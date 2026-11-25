"""The cut-off rule, with no Google dependencies so it can be tested anywhere."""
import json
from typing import Callable


def handle(payload: bytes, cut: Callable[[], object]) -> bool:
    """Call `cut` and return True when a budget notification reports cost at or over the budget.

    Anything unreadable or missing a figure is ignored: a bad message must never take the site down.
    """
    try:
        note = json.loads(payload)
        cost, budget = float(note["costAmount"]), float(note["budgetAmount"])
    except (ValueError, KeyError, TypeError):
        return False
    if cost < budget:
        return False
    cut()
    return True
