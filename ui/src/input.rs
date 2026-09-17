use gpui::Keystroke;

pub(crate) enum Input {
    Key { code: u32, down: bool },
    Button { button: u32, down: bool },
    Move { x: u32, y: u32 },
    Wheel { up: bool },
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Keyboard",
    default_path = "/org/qemu/Display1/Console_0"
)]
pub trait Keyboard {
    #[zbus(name = "Press")]
    fn press(&self, keycode: u32) -> zbus::Result<()>;

    #[zbus(name = "Release")]
    fn release(&self, keycode: u32) -> zbus::Result<()>;
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Mouse",
    default_path = "/org/qemu/Display1/Console_0"
)]
pub trait Mouse {
    #[zbus(name = "Press")]
    fn press(&self, button: u32) -> zbus::Result<()>;

    #[zbus(name = "Release")]
    fn release(&self, button: u32) -> zbus::Result<()>;

    #[zbus(name = "SetAbsPosition")]
    fn set_abs_position(&self, x: u32, y: u32) -> zbus::Result<()>;
}

pub(crate) const SHIFT: u32 = 0x2a;

pub(crate) const CONTROL: u32 = 0x1d;

pub(crate) const ALT: u32 = 0x38;

pub(crate) const SUPER: u32 = 0xdb;

pub(crate) const CAPS_LOCK: u32 = 0x3a;

pub(crate) const WHEEL_UP: u32 = 3;

pub(crate) const WHEEL_DOWN: u32 = 4;

pub(crate) fn is_settings_toggle(keystroke: &Keystroke) -> bool {
    (keystroke.modifiers.platform || keystroke.modifiers.control) && keystroke.key == ","
}

pub(crate) fn keycode(key: &str) -> Option<u32> {
    Some(match key {
        "escape" => 0x01,
        "1" => 0x02,
        "2" => 0x03,
        "3" => 0x04,
        "4" => 0x05,
        "5" => 0x06,
        "6" => 0x07,
        "7" => 0x08,
        "8" => 0x09,
        "9" => 0x0a,
        "0" => 0x0b,
        "-" => 0x0c,
        "=" => 0x0d,
        "backspace" => 0x0e,
        "tab" => 0x0f,
        "q" => 0x10,
        "w" => 0x11,
        "e" => 0x12,
        "r" => 0x13,
        "t" => 0x14,
        "y" => 0x15,
        "u" => 0x16,
        "i" => 0x17,
        "o" => 0x18,
        "p" => 0x19,
        "[" => 0x1a,
        "]" => 0x1b,
        "enter" => 0x1c,
        "a" => 0x1e,
        "s" => 0x1f,
        "d" => 0x20,
        "f" => 0x21,
        "g" => 0x22,
        "h" => 0x23,
        "j" => 0x24,
        "k" => 0x25,
        "l" => 0x26,
        ";" => 0x27,
        "'" => 0x28,
        "`" => 0x29,
        "\\" => 0x2b,
        "z" => 0x2c,
        "x" => 0x2d,
        "c" => 0x2e,
        "v" => 0x2f,
        "b" => 0x30,
        "n" => 0x31,
        "m" => 0x32,
        "," => 0x33,
        "." => 0x34,
        "/" => 0x35,
        "space" => 0x39,
        "capslock" => CAPS_LOCK,
        "f1" => 0x3b,
        "f2" => 0x3c,
        "f3" => 0x3d,
        "f4" => 0x3e,
        "f5" => 0x3f,
        "f6" => 0x40,
        "f7" => 0x41,
        "f8" => 0x42,
        "f9" => 0x43,
        "f10" => 0x44,
        "f11" => 0x57,
        "f12" => 0x58,
        "home" => 0x80 | 0x47,
        "up" => 0x80 | 0x48,
        "pageup" => 0x80 | 0x49,
        "left" => 0x80 | 0x4b,
        "right" => 0x80 | 0x4d,
        "end" => 0x80 | 0x4f,
        "down" => 0x80 | 0x50,
        "pagedown" => 0x80 | 0x51,
        "insert" => 0x80 | 0x52,
        "delete" => 0x80 | 0x53,
        _ => return None,
    })
}

pub(crate) async fn send_button(mouse: &MouseProxy<'_>, button: u32, down: bool) -> zbus::Result<()> {
    if down {
        mouse.press(button).await
    } else {
        mouse.release(button).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes_are_xt_set1_with_the_extended_bit() {
        assert_eq!(keycode("h"), Some(0x23));
        assert_eq!(keycode("a"), Some(0x1e));
        assert_eq!(keycode("escape"), Some(0x01));
        assert_eq!(keycode("space"), Some(0x39));
        assert_eq!(keycode("up"), Some(0x80 | 0x48));
        assert_eq!(keycode("delete"), Some(0x80 | 0x53));
        assert_eq!(keycode("nonsense"), None);
    }
}
