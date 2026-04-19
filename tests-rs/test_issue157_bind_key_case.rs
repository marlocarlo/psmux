use crossterm::event::{KeyCode, KeyModifiers};
use crate::config::{parse_key_name, parse_key_string, normalize_key_for_binding, format_key_binding};

/// Issue #157: bind-key should be case-sensitive for single character keys.
/// `bind-key T` must only fire on uppercase T (Shift+t), not lowercase t.

#[test]
fn parse_key_name_preserves_case_uppercase() {
    // parse_key_name("T") should yield Char('T'), not Char('t')
    let result = parse_key_name("T").unwrap();
    assert_eq!(result, (KeyCode::Char('T'), KeyModifiers::NONE),
        "parse_key_name should preserve uppercase 'T'");
}

#[test]
fn parse_key_name_preserves_case_lowercase() {
    let result = parse_key_name("t").unwrap();
    assert_eq!(result, (KeyCode::Char('t'), KeyModifiers::NONE),
        "parse_key_name should preserve lowercase 't'");
}

#[test]
fn parse_key_string_preserves_case_uppercase() {
    // The bug: parse_key_string("T") was returning Char('t') because it lowercased
    let result = parse_key_string("T").unwrap();
    assert_eq!(result, (KeyCode::Char('T'), KeyModifiers::NONE),
        "parse_key_string('T') should return Char('T'), not Char('t')");
}

#[test]
fn parse_key_string_preserves_case_lowercase() {
    let result = parse_key_string("t").unwrap();
    assert_eq!(result, (KeyCode::Char('t'), KeyModifiers::NONE),
        "parse_key_string('t') should return Char('t')");
}

#[test]
fn uppercase_and_lowercase_bindings_are_distinct() {
    // Simulate what happens during binding lookup:
    // Server stores bind-key T as (Char('T'), NONE) after normalization
    // Server stores bind-key t as (Char('t'), NONE) after normalization
    let binding_upper = normalize_key_for_binding(parse_key_name("T").unwrap());
    let binding_lower = normalize_key_for_binding(parse_key_name("t").unwrap());
    
    assert_ne!(binding_upper, binding_lower,
        "Bindings for 'T' and 't' must be distinct");
}

#[test]
fn roundtrip_format_parse_preserves_case() {
    // Server formats binding key to sync to client, client parses it back.
    // The case must survive the roundtrip.
    let original_upper = (KeyCode::Char('T'), KeyModifiers::NONE);
    let original_lower = (KeyCode::Char('t'), KeyModifiers::NONE);
    
    let formatted_upper = format_key_binding(&original_upper);
    let formatted_lower = format_key_binding(&original_lower);
    
    assert_eq!(formatted_upper, "T");
    assert_eq!(formatted_lower, "t");
    
    // Now parse them back (this is what the client does)
    let parsed_upper = parse_key_string(&formatted_upper).unwrap();
    let parsed_lower = parse_key_string(&formatted_lower).unwrap();
    
    assert_eq!(parsed_upper.0, KeyCode::Char('T'),
        "Roundtrip of uppercase 'T' must preserve case");
    assert_eq!(parsed_lower.0, KeyCode::Char('t'),
        "Roundtrip of lowercase 't' must preserve case");
}

#[test]
fn client_side_binding_match_uppercase_key() {
    // Simulate the client-side binding match flow:
    // 1. Server has bind-key T, formats as "T", syncs to client
    // 2. Client receives binding with k="T"
    // 3. User presses Shift+t -> crossterm: KeyCode::Char('T'), KeyModifiers::SHIFT
    // 4. User presses t -> crossterm: KeyCode::Char('t'), KeyModifiers::NONE
    
    let binding_key_str = "T"; // synced from server
    let parsed_binding = parse_key_string(binding_key_str).unwrap();
    let normalized_binding = normalize_key_for_binding(parsed_binding);
    
    // Simulate Shift+t keypress
    let shift_t_event = (KeyCode::Char('T'), KeyModifiers::SHIFT);
    let normalized_shift_t = normalize_key_for_binding(shift_t_event);
    
    // Simulate plain t keypress
    let plain_t_event = (KeyCode::Char('t'), KeyModifiers::NONE);
    let normalized_plain_t = normalize_key_for_binding(plain_t_event);
    
    assert_eq!(normalized_binding, normalized_shift_t,
        "Binding for 'T' should match Shift+t keypress");
    assert_ne!(normalized_binding, normalized_plain_t,
        "Binding for 'T' should NOT match plain 't' keypress");
}

#[test]
fn client_side_binding_match_lowercase_key() {
    let binding_key_str = "t"; // synced from server
    let parsed_binding = parse_key_string(binding_key_str).unwrap();
    let normalized_binding = normalize_key_for_binding(parsed_binding);
    
    // Simulate plain t keypress
    let plain_t_event = (KeyCode::Char('t'), KeyModifiers::NONE);
    let normalized_plain_t = normalize_key_for_binding(plain_t_event);
    
    // Simulate Shift+t keypress
    let shift_t_event = (KeyCode::Char('T'), KeyModifiers::SHIFT);
    let normalized_shift_t = normalize_key_for_binding(shift_t_event);
    
    assert_eq!(normalized_binding, normalized_plain_t,
        "Binding for 't' should match plain 't' keypress");
    assert_ne!(normalized_binding, normalized_shift_t,
        "Binding for 't' should NOT match Shift+t keypress");
}

#[test]
fn parse_key_string_all_letters_case_sensitive() {
    // Verify ALL letters preserve case, not just 't'/'T'
    for ch in 'A'..='Z' {
        let upper_str = ch.to_string();
        let lower_str = ch.to_ascii_lowercase().to_string();
        
        let parsed_upper = parse_key_string(&upper_str).unwrap();
        let parsed_lower = parse_key_string(&lower_str).unwrap();
        
        assert_eq!(parsed_upper.0, KeyCode::Char(ch),
            "parse_key_string('{}') should return Char('{}')", ch, ch);
        assert_eq!(parsed_lower.0, KeyCode::Char(ch.to_ascii_lowercase()),
            "parse_key_string('{}') should return Char('{}')", 
            ch.to_ascii_lowercase(), ch.to_ascii_lowercase());
    }
}

#[test]
fn named_keys_still_case_insensitive() {
    // Named keys like "Enter", "ENTER", "enter" should all work
    let e1 = parse_key_string("Enter").unwrap();
    let e2 = parse_key_string("ENTER").unwrap();
    let e3 = parse_key_string("enter").unwrap();
    assert_eq!(e1.0, KeyCode::Enter);
    assert_eq!(e2.0, KeyCode::Enter);
    assert_eq!(e3.0, KeyCode::Enter);
    
    let t1 = parse_key_string("Tab").unwrap();
    let t2 = parse_key_string("TAB").unwrap();
    assert_eq!(t1.0, KeyCode::Tab);
    assert_eq!(t2.0, KeyCode::Tab);
    
    let s1 = parse_key_string("Space").unwrap();
    let s2 = parse_key_string("SPACE").unwrap();
    assert_eq!(s1.0, KeyCode::Char(' '));
    assert_eq!(s2.0, KeyCode::Char(' '));
}
