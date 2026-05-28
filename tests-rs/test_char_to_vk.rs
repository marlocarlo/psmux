// char_to_vk gained explicit non-Win32 match arms for ESC ('\x1b' → 0x1B) and
// Enter ('\r' → 0x0D) so that send_modified_key_event works for those keys
// without VkKeyScanW returning -1 for them.

use super::mouse_inject::char_to_vk;

#[test]
fn escape_returns_vk_escape() {
    assert_eq!(char_to_vk('\x1b'), 0x1Bu16, "VK_ESCAPE");
}

#[test]
fn carriage_return_returns_vk_return() {
    assert_eq!(char_to_vk('\r'), 0x0Du16, "VK_RETURN");
}

#[test]
fn alphabetic_lowercase_maps_to_vk() {
    assert_eq!(char_to_vk('a'), 0x41u16, "VK_A");
    assert_eq!(char_to_vk('c'), 0x43u16, "VK_C");
    assert_eq!(char_to_vk('z'), 0x5Au16, "VK_Z");
}

#[test]
fn alphabetic_uppercase_same_as_lowercase() {
    // char_to_vk normalises to lowercase before calling VkKeyScanW
    assert_eq!(char_to_vk('A'), char_to_vk('a'));
    assert_eq!(char_to_vk('Z'), char_to_vk('z'));
}
