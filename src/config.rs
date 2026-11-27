//! O `config.toml`: o que cada botão faz, a velocidade do mouse, os combos.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use evdev::KeyCode;
use serde::Deserialize;

/// O arquivo que vem de fábrica, com um comentário em cada chave.
pub const DEFAULT: &str = include_str!("../pad.toml");

/// Os botões do controle, pelo nome do arquivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Select,
    Start,
    Home,
    Up,
    Down,
    Left,
    Right,
}

impl Button {
    pub const ALL: [Button; 17] = [
        Button::A,
        Button::B,
        Button::X,
        Button::Y,
        Button::L1,
        Button::R1,
        Button::L2,
        Button::R2,
        Button::L3,
        Button::R3,
        Button::Select,
        Button::Start,
        Button::Home,
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Button::A => "a",
            Button::B => "b",
            Button::X => "x",
            Button::Y => "y",
            Button::L1 => "l1",
            Button::R1 => "r1",
            Button::L2 => "l2",
            Button::R2 => "r2",
            Button::L3 => "l3",
            Button::R3 => "r3",
            Button::Select => "select",
            Button::Start => "start",
            Button::Home => "home",
            Button::Up => "up",
            Button::Down => "down",
            Button::Left => "left",
            Button::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Result<Button> {
        let s = s.trim().to_ascii_lowercase();
        Button::ALL
            .into_iter()
            .find(|b| b.name() == s)
            .ok_or_else(|| anyhow!("botão desconhecido: {s:?}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Left,
    Right,
    Middle,
}

/// O que um botão (ou um combo) faz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Talk,
    /// Clica onde está o mouse e dispara o ditado: o texto cai naquela caixa.
    Dictate,
    Focus,
    /// Teclas; `true` traz o T3 para a frente antes.
    Keys(Vec<KeyCode>, bool),
    Click(Click),
    /// Rola enquanto o botão está apertado: +1 para cima, -1 para baixo.
    Scroll(i8),
    Run(String),
    Toggle,
}

impl Action {
    pub fn parse(s: &str) -> Result<Action> {
        let s = s.trim();
        let (verb, rest) = s
            .split_once(char::is_whitespace)
            .map_or((s, ""), |(v, r)| (v, r.trim()));
        Ok(match verb.to_ascii_lowercase().as_str() {
            "" | "none" => Action::None,
            "talk" => Action::Talk,
            "dictate" => Action::Dictate,
            "focus" => Action::Focus,
            "toggle" => Action::Toggle,
            "key" => Action::Keys(parse_keys(rest)?, false),
            "t3" => Action::Keys(parse_keys(rest)?, true),
            "click" => Action::Click(match rest.to_ascii_lowercase().as_str() {
                "" | "left" => Click::Left,
                "right" => Click::Right,
                "middle" => Click::Middle,
                other => bail!("clique desconhecido: {other:?}"),
            }),
            "scroll" => Action::Scroll(match rest.to_ascii_lowercase().as_str() {
                "up" => 1,
                "down" => -1,
                other => bail!("rolagem desconhecida: {other:?} (up ou down)"),
            }),
            "run" if !rest.is_empty() => Action::Run(rest.to_string()),
            _ => bail!("ação desconhecida: {s:?}"),
        })
    }
}

/// "ctrl+shift+[" → [LEFTCTRL, LEFTSHIFT, LEFTBRACE].
pub fn parse_keys(s: &str) -> Result<Vec<KeyCode>> {
    if s.is_empty() {
        bail!("faltam as teclas");
    }
    // "+" sozinho no fim é a tecla de mais: "ctrl++".
    let mut parts: Vec<&str> = s.split('+').collect();
    if s.ends_with("++") {
        parts.truncate(parts.len() - 2);
        parts.push("+");
    }
    parts.into_iter().map(|p| key(p.trim())).collect()
}

fn key(name: &str) -> Result<KeyCode> {
    if name.starts_with("KEY_") || name.starts_with("BTN_") {
        return KeyCode::from_str(name).map_err(|_| anyhow!("tecla desconhecida: {name:?}"));
    }
    let lower = name.to_ascii_lowercase();
    let k = match lower.as_str() {
        "ctrl" | "control" => KeyCode::KEY_LEFTCTRL,
        "shift" => KeyCode::KEY_LEFTSHIFT,
        "alt" => KeyCode::KEY_LEFTALT,
        "altgr" => KeyCode::KEY_RIGHTALT,
        "super" | "meta" | "win" => KeyCode::KEY_LEFTMETA,
        "enter" | "return" => KeyCode::KEY_ENTER,
        "esc" | "escape" => KeyCode::KEY_ESC,
        "tab" => KeyCode::KEY_TAB,
        "space" => KeyCode::KEY_SPACE,
        "backspace" => KeyCode::KEY_BACKSPACE,
        "delete" | "del" => KeyCode::KEY_DELETE,
        "insert" => KeyCode::KEY_INSERT,
        "up" => KeyCode::KEY_UP,
        "down" => KeyCode::KEY_DOWN,
        "left" => KeyCode::KEY_LEFT,
        "right" => KeyCode::KEY_RIGHT,
        "home" => KeyCode::KEY_HOME,
        "end" => KeyCode::KEY_END,
        "pageup" => KeyCode::KEY_PAGEUP,
        "pagedown" => KeyCode::KEY_PAGEDOWN,
        "[" => KeyCode::KEY_LEFTBRACE,
        "]" => KeyCode::KEY_RIGHTBRACE,
        "-" | "minus" => KeyCode::KEY_MINUS,
        "=" | "equal" => KeyCode::KEY_EQUAL,
        "+" | "plus" => KeyCode::KEY_EQUAL,
        ";" => KeyCode::KEY_SEMICOLON,
        "'" => KeyCode::KEY_APOSTROPHE,
        "," | "comma" => KeyCode::KEY_COMMA,
        "." | "dot" => KeyCode::KEY_DOT,
        "/" | "slash" => KeyCode::KEY_SLASH,
        "\\" | "backslash" => KeyCode::KEY_BACKSLASH,
        "`" | "grave" => KeyCode::KEY_GRAVE,
        "volumeup" => KeyCode::KEY_VOLUMEUP,
        "volumedown" => KeyCode::KEY_VOLUMEDOWN,
        "mute" => KeyCode::KEY_MUTE,
        "playpause" => KeyCode::KEY_PLAYPAUSE,
        _ => {
            let up = lower.to_ascii_uppercase();
            let is_simple = (up.len() == 1 && up.chars().all(|c| c.is_ascii_alphanumeric()))
                || (up.starts_with('F')
                    && up[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
            if !is_simple {
                bail!("tecla desconhecida: {name:?}");
            }
            KeyCode::from_str(&format!("KEY_{up}"))
                .map_err(|_| anyhow!("tecla desconhecida: {name:?}"))?
        }
    };
    Ok(k)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    device: String,
    #[serde(default)]
    talk: RawTalk,
    #[serde(default)]
    mouse: Mouse,
    #[serde(default)]
    scroll: Scroll,
    #[serde(default)]
    notify: Notify,
    #[serde(default)]
    buttons: HashMap<String, String>,
    #[serde(default)]
    combos: HashMap<String, Vec<String>>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTalk {
    command: String,
}

impl Default for RawTalk {
    fn default() -> Self {
        RawTalk {
            command: "talk-rs toggle -l auto".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Mouse {
    pub sticks: Vec<String>,
    pub speed: f64,
    pub deadzone: f64,
    pub curve: f64,
}

impl Default for Mouse {
    fn default() -> Self {
        Mouse {
            sticks: vec!["left".into(), "right".into()],
            speed: 1500.0,
            deadzone: 0.15,
            curve: 2.2,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Scroll {
    pub enabled: bool,
    pub speed: f64,
}

impl Default for Scroll {
    fn default() -> Self {
        Scroll {
            enabled: true,
            speed: 25.0,
        }
    }
}

/// A vibração quando o T3 precisa de você.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Notify {
    pub enabled: bool,
    pub finished: bool,
    pub question: bool,
    pub pulses: u32,
    pub strength: f64,
    pub length_ms: u32,
}

impl Default for Notify {
    fn default() -> Self {
        Notify {
            enabled: true,
            finished: true,
            question: true,
            pulses: 2,
            strength: 0.6,
            length_ms: 150,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Combo {
    pub buttons: Vec<Button>,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub device: String,
    pub talk_command: String,
    pub left_stick_mouse: bool,
    pub right_stick_mouse: bool,
    pub mouse: Mouse,
    pub scroll: Scroll,
    pub notify: Notify,
    pub buttons: HashMap<Button, Action>,
    pub combos: Vec<Combo>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config> {
        let raw: Raw = toml::from_str(text)?;
        let mut buttons = HashMap::new();
        for (name, action) in &raw.buttons {
            let b = Button::parse(name)?;
            let a = Action::parse(action).with_context(|| format!("no botão {name}"))?;
            buttons.insert(b, a);
        }
        let mut combos = Vec::new();
        for (action, names) in &raw.combos {
            let action = Action::parse(action).with_context(|| format!("no combo {action:?}"))?;
            let buttons = names
                .iter()
                .map(|n| Button::parse(n))
                .collect::<Result<Vec<_>>>()?;
            if buttons.len() < 2 {
                bail!("o combo {action:?} precisa de pelo menos 2 botões");
            }
            combos.push(Combo { buttons, action });
        }
        // Combos maiores primeiro: quem aperta 4 botões não quer o de 2.
        combos.sort_by_key(|c| std::cmp::Reverse(c.buttons.len()));
        let mut sticks = raw.mouse.sticks.iter().map(|s| s.to_ascii_lowercase());
        let (mut left, mut right) = (false, false);
        for s in sticks.by_ref() {
            match s.as_str() {
                "left" => left = true,
                "right" => right = true,
                other => bail!("analógico desconhecido em [mouse].sticks: {other:?}"),
            }
        }
        let mouse = raw.mouse;
        if !(0.0..0.95).contains(&mouse.deadzone) || mouse.curve <= 0.0 || mouse.speed < 0.0 {
            bail!("[mouse]: deadzone entre 0 e 0.95, curve e speed positivos");
        }
        let n = &raw.notify;
        if !(1..=5).contains(&n.pulses) || !(0.0..=1.0).contains(&n.strength) || !(30..=1000).contains(&n.length_ms) {
            bail!("[notify]: pulses de 1 a 5, strength de 0 a 1, length_ms de 30 a 1000");
        }
        Ok(Config {
            enabled: raw.enabled,
            device: raw.device,
            talk_command: raw.talk.command,
            left_stick_mouse: left,
            right_stick_mouse: right,
            mouse,
            scroll: raw.scroll,
            notify: raw.notify,
            buttons,
            combos,
        })
    }

    /// Lê o arquivo; se não existe, escreve o de fábrica.
    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, DEFAULT)?;
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("lendo {}", path.display()))?;
        Config::parse(&text).with_context(|| format!("em {}", path.display()))
    }

    pub fn action(&self, b: Button) -> &Action {
        self.buttons.get(&b).unwrap_or(&Action::None)
    }

    /// O botão faz parte de algum combo (então dispara ao soltar)?
    pub fn in_combo(&self, b: Button) -> bool {
        self.combos.iter().any(|c| c.buttons.contains(&b))
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("pad").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses() {
        let c = Config::parse(DEFAULT).unwrap();
        assert_eq!(c.action(Button::L1), &Action::Talk);
        assert_eq!(c.action(Button::L3), &Action::Dictate);
        assert_eq!(
            c.action(Button::Up),
            &Action::Keys(
                vec![
                    KeyCode::KEY_LEFTCTRL,
                    KeyCode::KEY_LEFTSHIFT,
                    KeyCode::KEY_LEFTBRACE
                ],
                true
            )
        );
        assert!(c.in_combo(Button::Start));
        assert!(!c.in_combo(Button::A));
        assert!(c.left_stick_mouse && c.right_stick_mouse);
        assert_eq!(c.action(Button::Left), &Action::Scroll(1));
        assert_eq!(c.action(Button::Right), &Action::Scroll(-1));
    }

    #[test]
    fn keys() {
        assert_eq!(
            parse_keys("ctrl++").unwrap(),
            vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_EQUAL]
        );
        assert_eq!(parse_keys("f5").unwrap(), vec![KeyCode::KEY_F5]);
        assert_eq!(
            parse_keys("super+d").unwrap(),
            vec![KeyCode::KEY_LEFTMETA, KeyCode::KEY_D]
        );
        assert_eq!(
            parse_keys("KEY_RIGHTBRACE").unwrap(),
            vec![KeyCode::KEY_RIGHTBRACE]
        );
        assert!(parse_keys("banana").is_err());
    }
}
