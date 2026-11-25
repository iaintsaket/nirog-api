import json
import unittest

from guard import handle


def note(cost, budget):
    return json.dumps({"costAmount": cost, "budgetAmount": budget, "currencyCode": "INR"}).encode()


class CutBillingAtBudget(unittest.TestCase):
    def test_cuts_only_once_cost_reaches_the_budget(self):
        cuts = []
        self.assertFalse(handle(note(49.99, 50), lambda: cuts.append(1)))
        self.assertTrue(handle(note(50, 50), lambda: cuts.append(1)))
        self.assertTrue(handle(note(80.5, 50), lambda: cuts.append(1)))
        self.assertEqual(len(cuts), 2)

    def test_a_malformed_message_never_cuts(self):
        cuts = []
        for bad in (b"", b"not json", b"{}", json.dumps({"costAmount": 99}).encode()):
            self.assertFalse(handle(bad, lambda: cuts.append(1)))
        self.assertEqual(cuts, [])


if __name__ == "__main__":
    unittest.main()
