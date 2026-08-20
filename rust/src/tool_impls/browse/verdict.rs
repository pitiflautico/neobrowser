//! What a click's result *means*: dispatched or not, blocked or failed, attributable or not.
//!
//! Separated from the tool because this is where the verified-action contract is actually
//! decided, and it deserves to be readable without the argument handling around it. Every
//! rule here was put in by a failure that reached a real page or a conformance scenario, and
//! each one is documented with which.

use crate::action::ActionStatus;
use crate::cdp::CdpClient;
use crate::page;

/// Whether the click left the gate, and the sentence describing what happened.
pub(super) fn describe(outcome: &page::ClickOutcome) -> (bool, String, ActionStatus) {
    // A click that never left the gate is decided by the outcome alone — there is
    // no point waiting for a page reaction to an event we did not dispatch.
    let dispatched = match &outcome {
        page::ClickOutcome::Clicked | page::ClickOutcome::NoLayoutUsedJs => true,
        page::ClickOutcome::NotFound
        | page::ClickOutcome::Obscured { .. }
        | page::ClickOutcome::Disabled { .. } => false,
    };
    let detail = match &outcome {
        page::ClickOutcome::Clicked => "click dispatched as real mouse events".to_string(),
        page::ClickOutcome::NoLayoutUsedJs => {
            "click dispatched via JS fallback (element had no box model)".to_string()
        }
        page::ClickOutcome::NotFound => "click target not found".to_string(),
        page::ClickOutcome::Obscured { by } => format!(
            "not clicked: target is covered by {by}. Dismiss the overlay \
             (dismiss_overlay) or scroll it out of the way, then retry"
        ),
        page::ClickOutcome::Disabled { reason } => format!(
            "not clicked: {reason}. Change what keeps it disabled — a required field, a \
             pending validation — rather than retrying the click"
        ),
    };
    // An obstruction is not a failure, and the difference is the caller's next move.
    //
    // `failed` means "this did not happen and will not on retry"; `blocked` means "clear
    // the thing in the way and try again". The distinction was already encoded here — in
    // `retryable`, and in the detail text — but the *status* lumped an overlay in with a
    // target that does not exist. A caller switching on status therefore read a removable
    // cookie banner as a dead end. Conformance scenario C2 is what caught it.
    let status = match &outcome {
        page::ClickOutcome::Obscured { .. } | page::ClickOutcome::Disabled { .. } => {
            ActionStatus::Blocked
        }
        _ => ActionStatus::Failed,
    };

    (dispatched, detail, status)
}

/// Whether the observed change can be credited to this click.
///
/// `quiet` is whether the page had settled before the action. That is the load-bearing input:
/// on a settled page anything that moves afterwards is the click's, which is what quiescence
/// buys. Only when the page refuses to settle does the evidence have to earn it — and then it
/// earns it structurally (something appeared, vanished or toggled, or the page navigated) or by
/// the clicked element itself having changed.
pub(super) async fn attributable(
    client: &CdpClient,
    target_id: Option<i64>,
    fingerprint_before: &Option<String>,
    observed: &[String],
    quiet: bool,
) -> bool {
    let fingerprint_after = match target_id {
        Some(id) => page::element_fingerprint(client, id).await,
        None => None,
    };
    let target_changed = match (fingerprint_before, &fingerprint_after) {
        // Gone from the DOM: a click that removed its own target did something.
        (Some(_), None) => true,
        (Some(a), Some(b)) => a != b,
        // No fingerprint either side — nothing to conclude from, so lean on the rest.
        _ => false,
    };
    let structural = observed
        .iter()
        .any(|c| c == "navigation" || c == "dom_nodes" || c == "control_state");
    quiet || structural || target_changed
}

/// The status for a click that was dispatched, given what the page did about it.
pub(super) fn dispatched_status(changed: bool, attributable: bool) -> ActionStatus {
    if changed && attributable {
        ActionStatus::Succeeded
    } else {
        ActionStatus::Uncertain
    }
}
