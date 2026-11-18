use super::*;

#[test]
fn the_link_opens_the_pane_and_carries_no_card_content() {
    assert_eq!(
        pane_link("https://chariox.example/", "room 1", "agent-1"),
        "https://chariox.example/terminal?session=room+1&agent=agent-1"
    );
}

#[test]
fn the_title_names_the_room_the_agent_the_waiting_cards_and_the_goal() {
    assert_eq!(title_line("Release", "writer", 0, None), "Release · writer");
    assert_eq!(
        title_line("Release", "writer", 1, Some("Ship the notes")),
        "Release · writer · 1 card waiting · Ship the notes"
    );
    assert_eq!(
        title_line("Release", "writer", 3, None),
        "Release · writer · 3 cards waiting"
    );
}

#[test]
fn delivery_bookkeeping_is_no_blocked_stripe() {
    assert!(is_delivery_bookkeeping("Unconfirmed delivery 3"));
    assert!(is_delivery_bookkeeping("Delivery 2 refusal clock runs"));
    assert!(!is_delivery_bookkeeping("Waiting for the review"));
}

fn card(choice: crate::session::RuntimeInteractionChoice) -> RuntimeInteraction {
    RuntimeInteraction::new(
        "card",
        "agent",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Proceed?",
        vec![choice],
        None,
        None,
        None,
    )
}

#[test]
fn a_text_terminal_answers_choices_but_a_passkey_choice_opens_the_pane() {
    let allow = crate::session::RuntimeInteractionChoice::new("allow", "Allow", "allow", None);
    assert!(text_terminal_answers(&card(allow.clone())));
    assert!(!text_terminal_answers(&card(allow.requiring_passkey())));
}
