//! In-memory fakes of every port, for use-case tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use nirog_domain::{
    Form, MatchKey, Offer, OfferId, Pack, PackUnit, Paise, Release, Salt, SourceKind,
};
use time::{Duration, OffsetDateTime, macros::datetime};
use uuid::Uuid;

use crate::error::{IdentityError, RepoError};
use crate::model::{Identity, NewSavedMatch, SavedMatch, User, UserId};
use crate::ports::{
    Clock, IdentityProvider, LoginStateRepository, OfferRepository, PriceSource,
    SavedMatchRepository, SessionRepository, SourceError, UserRepository,
};
use crate::security::TokenHash;

pub fn key(name: &str) -> MatchKey {
    MatchKey::new(
        &[Salt::mg(name, 1000)],
        Form::Tablet,
        Release::Standard,
        PackUnit::Unit,
    )
    .unwrap()
}

pub fn offer(id: &str, source: SourceKind, name: &str, price: i64, key_name: &str) -> Offer {
    Offer {
        id: OfferId(id.into()),
        source,
        name: name.into(),
        manufacturer: "Maker".into(),
        price: Paise(price),
        pack: Pack {
            count: 10,
            unit: PackUnit::Unit,
        },
        key: key(key_name),
        salts_label: format!("{key_name} 1mg"),
    }
}

#[derive(Debug, Default)]
pub struct FakeOffers(Mutex<Vec<Offer>>);

impl FakeOffers {
    pub fn with(offers: Vec<Offer>) -> Self {
        Self(Mutex::new(offers))
    }
}

#[async_trait]
impl OfferRepository for FakeOffers {
    async fn closest_names(&self, query: &str, limit: u32) -> Result<Vec<String>, RepoError> {
        // Stand-in for trigram similarity: same first three letters.
        let head: String = query.to_lowercase().chars().take(3).collect();
        let all = self.0.lock().unwrap();
        let mut names: Vec<String> = all
            .iter()
            .map(|o| o.name.clone())
            .filter(|n| n.to_lowercase().starts_with(&head))
            .collect();
        names.dedup();
        names.truncate(limit as usize);
        Ok(names)
    }

    async fn search(&self, query: &str, limit: u32) -> Result<Vec<Offer>, RepoError> {
        let q = query.to_lowercase();
        let all = self.0.lock().unwrap();
        Ok(all
            .iter()
            .filter(|o| {
                o.name.to_lowercase().contains(&q) || o.salts_label.to_lowercase().contains(&q)
            })
            .take(limit as usize)
            .cloned()
            .collect())
    }
    async fn by_key(&self, key: &MatchKey) -> Result<Vec<Offer>, RepoError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|o| &o.key == key)
            .cloned()
            .collect())
    }
    async fn key_exists(&self, key: &MatchKey) -> Result<bool, RepoError> {
        Ok(self.0.lock().unwrap().iter().any(|o| &o.key == key))
    }
    async fn replace_source(&self, source: SourceKind, offers: &[Offer]) -> Result<u64, RepoError> {
        let mut all = self.0.lock().unwrap();
        all.retain(|o| o.source != source);
        all.extend_from_slice(offers);
        Ok(offers.len() as u64)
    }
}

#[derive(Debug)]
pub struct FakeClock(Mutex<OffsetDateTime>);

impl Default for FakeClock {
    fn default() -> Self {
        Self(Mutex::new(datetime!(2026-01-01 0:00 UTC)))
    }
}

impl FakeClock {
    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        *self.0.lock().unwrap()
    }
}

#[derive(Debug)]
pub struct FakeIdp(bool);

impl FakeIdp {
    pub fn ok() -> Self {
        Self(true)
    }
    pub fn failing() -> Self {
        Self(false)
    }
}

#[async_trait]
impl IdentityProvider for FakeIdp {
    fn authorize_url(&self, state: &str, challenge: &str) -> String {
        format!("https://idp.test/auth?state={state}&code_challenge={challenge}")
    }
    async fn exchange(&self, _code: &str, _verifier: &str) -> Result<Identity, IdentityError> {
        if self.0 {
            Ok(Identity {
                provider: "fake",
                subject: "sub-1".into(),
                email: "asha@example.com".into(),
                name: "Asha".into(),
            })
        } else {
            Err(IdentityError::Rejected("bad code".into()))
        }
    }
}

/// Users, sessions and login states in one store so deletes can cascade.
#[derive(Debug, Default)]
pub struct InMemoryAuth {
    users: Mutex<HashMap<Uuid, (String, User)>>,
    sessions: Mutex<HashMap<TokenHash, (UserId, OffsetDateTime)>>,
    states: Mutex<HashMap<TokenHash, (String, OffsetDateTime)>>,
}

#[async_trait]
impl UserRepository for InMemoryAuth {
    async fn upsert_identity(&self, id: &Identity) -> Result<User, RepoError> {
        let mut users = self.users.lock().unwrap();
        let subject = format!("{}:{}", id.provider, id.subject);
        if let Some((_, u)) = users.values().find(|(s, _)| *s == subject) {
            return Ok(u.clone());
        }
        let user = User {
            id: UserId(Uuid::new_v4()),
            email: id.email.clone(),
            name: id.name.clone(),
        };
        users.insert(user.id.0, (subject, user.clone()));
        Ok(user)
    }
    async fn get(&self, id: UserId) -> Result<Option<User>, RepoError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .get(&id.0)
            .map(|(_, u)| u.clone()))
    }
    async fn delete(&self, id: UserId) -> Result<bool, RepoError> {
        self.sessions.lock().unwrap().retain(|_, (u, _)| *u != id);
        Ok(self.users.lock().unwrap().remove(&id.0).is_some())
    }
}

#[async_trait]
impl SessionRepository for InMemoryAuth {
    async fn create(
        &self,
        hash: &TokenHash,
        user: UserId,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(hash.clone(), (user, expires_at));
        Ok(())
    }
    async fn user_for(
        &self,
        hash: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<UserId>, RepoError> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(hash)
            .filter(|(_, exp)| *exp > now)
            .map(|(u, _)| *u))
    }
    async fn delete(&self, hash: &TokenHash) -> Result<(), RepoError> {
        self.sessions.lock().unwrap().remove(hash);
        Ok(())
    }
}

#[async_trait]
impl LoginStateRepository for InMemoryAuth {
    async fn put(
        &self,
        state: &TokenHash,
        verifier: &str,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError> {
        self.states
            .lock()
            .unwrap()
            .insert(state.clone(), (verifier.into(), expires_at));
        Ok(())
    }
    async fn take(
        &self,
        state: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<String>, RepoError> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .remove(state)
            .filter(|(_, exp)| *exp > now)
            .map(|(v, _)| v))
    }
}

#[derive(Debug, Default)]
pub struct FakeSaved(Mutex<Vec<(UserId, SavedMatch)>>);

impl FakeSaved {
    pub fn fill(&self, user: UserId, n: u64) {
        let mut all = self.0.lock().unwrap();
        for i in 0..n {
            all.push((
                user,
                SavedMatch {
                    id: Uuid::new_v4(),
                    key: key(&format!("f{i}")),
                    offer_id: None,
                    label: "x".into(),
                    created_at: OffsetDateTime::now_utc(),
                },
            ));
        }
    }
}

#[async_trait]
impl SavedMatchRepository for FakeSaved {
    async fn list(&self, user: UserId) -> Result<Vec<SavedMatch>, RepoError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(u, _)| *u == user)
            .map(|(_, s)| s.clone())
            .collect())
    }
    async fn count(&self, user: UserId) -> Result<u64, RepoError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(u, _)| *u == user)
            .count() as u64)
    }
    async fn save(&self, user: UserId, item: &NewSavedMatch) -> Result<SavedMatch, RepoError> {
        let s = SavedMatch {
            id: Uuid::new_v4(),
            key: item.key.clone(),
            offer_id: item.offer_id.clone(),
            label: item.label.clone(),
            created_at: OffsetDateTime::now_utc(),
        };
        self.0.lock().unwrap().push((user, s.clone()));
        Ok(s)
    }
    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool, RepoError> {
        let mut all = self.0.lock().unwrap();
        let before = all.len();
        all.retain(|(u, s)| !(*u == user && s.id == id));
        Ok(all.len() != before)
    }
}

#[derive(Debug)]
pub struct FakeSource(pub Result<Vec<Offer>, String>);

impl PriceSource for FakeSource {
    fn kind(&self) -> SourceKind {
        SourceKind::JanAushadhi
    }
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError> {
        self.0.clone().map(|o| (o, 2)).map_err(SourceError)
    }
}
