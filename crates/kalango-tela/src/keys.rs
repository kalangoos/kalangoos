//! Keys for the virtual keyboard: a keymap made for each request (only the symbols it needs, one keycode each, so
//! any character types without depending on the person's layout) and the shortcuts written as "ctrl+s".

/// Modifier keys, always at the first keycodes: (name, xkb keycode, keysym, modifier map, mask).
pub const MODIFIERS: [(&str, u32, &str, &str, u32); 4] = [
    ("shift", 9, "Shift_L", "Shift", 1),
    ("ctrl", 10, "Control_L", "Control", 4),
    ("alt", 11, "Alt_L", "Mod1", 8),
    ("super", 12, "Super_L", "Mod4", 64),
];

/// The first keycode for the request's symbols (after the modifiers); xkb keycodes go up to 255.
const FIRST: u32 = 13;
pub const MAX_SYMBOLS: usize = (255 - FIRST + 1) as usize;

/// The keysym name of a character: letters and digits by their own name, a few by theirs, the rest as Unicode.
pub fn keysym_of(c: char) -> String {
    match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => c.to_string(),
        ' ' => "space".into(),
        '\n' => "Return".into(),
        '\t' => "Tab".into(),
        _ => format!("U{:04X}", c as u32),
    }
}

/// A shortcut: the modifiers (names of `MODIFIERS`) and the key's keysym.
#[derive(Debug, Clone, PartialEq)]
pub struct Combo {
    pub modifiers: Vec<&'static str>,
    pub keysym: String,
}

/// "ctrl+shift+s", "Enter", "esc", "alt+F4", "ctrl+plus".
pub fn parse_combo(s: &str) -> Result<Combo, String> {
    let parts: Vec<String> = s.split('+').map(|p| p.trim().to_lowercase()).collect();
    if parts.iter().any(|p| p.is_empty()) {
        // "ctrl++" is ctrl and the plus key
        if s.trim().ends_with("++") || s.trim() == "+" {
            let head = s.trim().trim_end_matches('+');
            let mut c = if head.is_empty() { Combo { modifiers: vec![], keysym: String::new() } } else { parse_mods(head)? };
            c.keysym = "plus".into();
            return Ok(c);
        }
        return Err(format!("invalid shortcut: {s:?}"));
    }
    let (key, mods) = parts.split_last().ok_or_else(|| format!("invalid shortcut: {s:?}"))?;
    let mut modifiers = Vec::new();
    for m in mods {
        modifiers.push(modifier(m).ok_or_else(|| format!("unknown modifier: {m:?}"))?);
    }
    // a modifier alone ("shift") is a key too
    let keysym = match modifier(key) {
        Some(m) => MODIFIERS.iter().find(|x| x.0 == m).map(|x| x.2.to_string()).unwrap_or_default(),
        None => key_name(key).ok_or_else(|| format!("unknown key: {key:?}"))?,
    };
    Ok(Combo { modifiers, keysym })
}

fn parse_mods(s: &str) -> Result<Combo, String> {
    let mut modifiers = Vec::new();
    for m in s.split('+') {
        modifiers.push(modifier(&m.trim().to_lowercase()).ok_or_else(|| format!("unknown modifier: {m:?}"))?);
    }
    Ok(Combo { modifiers, keysym: String::new() })
}

fn modifier(m: &str) -> Option<&'static str> {
    Some(match m {
        "ctrl" | "control" | "ctl" => "ctrl",
        "shift" => "shift",
        "alt" | "option" => "alt",
        "super" | "win" | "meta" | "logo" | "cmd" => "super",
        _ => return None,
    })
}

fn key_name(k: &str) -> Option<String> {
    let named = match k {
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "space" | "spacebar" => "space",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" | "pgup" => "Prior",
        "pagedown" | "page_down" | "pgdn" => "Next",
        "menu" => "Menu",
        "plus" => "plus",
        "minus" => "minus",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.into());
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
        return Some(format!("F{n}"));
    }
    let mut cs = k.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) => Some(keysym_of(c)),
        _ => None,
    }
}

/// The keymap text with the modifiers and these symbols (keycode `FIRST + i` for the i-th).
pub fn keymap(symbols: &[String]) -> String {
    let mut codes = String::new();
    let mut syms = String::new();
    let mut map = String::new();
    for (name, code, sym, modmap, _) in MODIFIERS {
        codes.push_str(&format!("<M{name}> = {code};\n"));
        syms.push_str(&format!("key <M{name}> {{ [ {sym} ] }};\n"));
        map.push_str(&format!("modifier_map {modmap} {{ <M{name}> }};\n"));
    }
    for (i, s) in symbols.iter().enumerate() {
        let code = FIRST + i as u32;
        codes.push_str(&format!("<K{code}> = {code};\n"));
        syms.push_str(&format!("key <K{code}> {{ [ {s} ] }};\n"));
    }
    format!("xkb_keymap {{\nxkb_keycodes \"kalango\" {{ minimum = 8; maximum = 255;\n{codes}}};\nxkb_types \"kalango\" {{ include \"complete\" }};\nxkb_compat \"kalango\" {{ include \"complete\" }};\nxkb_symbols \"kalango\" {{\n{syms}{map}}};\n}};\n")
}

/// The evdev code (xkb keycode less 8) of the i-th symbol.
pub fn evdev_of_symbol(i: usize) -> u32 {
    FIRST + i as u32 - 8
}

pub fn modifier_evdev(name: &str) -> (u32, u32) {
    MODIFIERS.iter().find(|m| m.0 == name).map(|m| (m.1 - 8, m.4)).unwrap_or((0, 0))
}

/// The text cut in pieces of at most `MAX_SYMBOLS` different symbols (each piece gets its own keymap).
pub fn chunks(text: &str) -> Vec<(Vec<String>, Vec<usize>)> {
    let mut out: Vec<(Vec<String>, Vec<usize>)> = Vec::new();
    let mut syms: Vec<String> = Vec::new();
    let mut seq: Vec<usize> = Vec::new();
    for c in text.chars() {
        if c == '\r' {
            continue;
        }
        let k = keysym_of(c);
        let i = match syms.iter().position(|s| *s == k) {
            Some(i) => i,
            None => {
                if syms.len() == MAX_SYMBOLS {
                    out.push((std::mem::take(&mut syms), std::mem::take(&mut seq)));
                }
                syms.push(k);
                syms.len() - 1
            }
        };
        seq.push(i);
    }
    if !seq.is_empty() {
        out.push((syms, seq));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts() {
        assert_eq!(parse_combo("ctrl+s").unwrap(), Combo { modifiers: vec!["ctrl"], keysym: "s".into() });
        assert_eq!(parse_combo("Ctrl+Shift+T").unwrap(), Combo { modifiers: vec!["ctrl", "shift"], keysym: "t".into() });
        assert_eq!(parse_combo("Enter").unwrap().keysym, "Return");
        assert_eq!(parse_combo("alt+F4").unwrap(), Combo { modifiers: vec!["alt"], keysym: "F4".into() });
        assert_eq!(parse_combo("ctrl++").unwrap(), Combo { modifiers: vec!["ctrl"], keysym: "plus".into() });
        assert_eq!(parse_combo("pagedown").unwrap().keysym, "Next");
        assert_eq!(parse_combo("shift").unwrap().keysym, "Shift_L");
        assert!(parse_combo("hyper+x").is_err());
        assert!(parse_combo("ctrl+nada").is_err());
    }

    #[test]
    fn characters_and_keymap() {
        assert_eq!(keysym_of('a'), "a");
        assert_eq!(keysym_of('ç'), "U00E7");
        assert_eq!(keysym_of('\n'), "Return");
        let c = chunks("olá olá");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, vec!["o", "l", "U00E1", "space"]);
        assert_eq!(c[0].1, vec![0, 1, 2, 3, 0, 1, 2]);
        let k = keymap(&c[0].0);
        assert!(k.contains("<K13> = 13;") && k.contains("key <K15> { [ U00E1 ] };") && k.contains("modifier_map Control { <Mctrl> };"));
        assert_eq!(evdev_of_symbol(0), 5);
        // more different characters than keycodes: several keymaps
        let many: String = (0..300u32).filter_map(|i| char::from_u32(0x4E00 + i)).collect();
        let c = chunks(&many);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0.len(), MAX_SYMBOLS);
    }
}
