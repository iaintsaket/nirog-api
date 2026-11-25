"""Cloud Run function: unlinks billing from the project once the budget is reached.

Pattern from https://docs.cloud.google.com/billing/docs/how-to/disable-billing-with-notifications
Deployed by ../budget-guard.sh. With billing unlinked, paid services stop and nothing more is charged.
"""
import base64
import os

import functions_framework
from google.cloud import billing_v1

from guard import handle

PROJECT_ID = os.environ["PROJECT_ID"]


def _unlink_billing() -> None:
    billing_v1.CloudBillingClient().update_project_billing_info(
        name=f"projects/{PROJECT_ID}",
        project_billing_info=billing_v1.ProjectBillingInfo(billing_account_name=""),
    )
    print(f"Budget reached: billing unlinked from {PROJECT_ID}")


@functions_framework.cloud_event
def stop_billing(event) -> None:
    handle(base64.b64decode(event.data["message"]["data"]), _unlink_billing)
