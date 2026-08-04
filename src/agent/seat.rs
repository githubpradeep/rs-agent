//! Named seats — persistent identity across sessions.
//!
//! A *session* is a day; a *seat* is a person (role + history) that survives
//! model upgrades and renames.

use crate::agent::handoff::HandoffNotes;
use crate::agent::laurel::Laurel;
use crate::beads::BeadKind;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// City caste — enforces which bead kinds a seat may claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SeatCaste {
    Crew,
    Fleet,
    Review,
    Marshal,
    Seneschal,
    Role,
    #[default]
    Any,
}

impl SeatCaste {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::Fleet => "fleet",
            Self::Review => "review",
            Self::Marshal => "marshal",
            Self::Seneschal => "seneschal",
            Self::Role => "role",
            Self::Any => "any",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "crew" | "designer" | "designers" => Some(Self::Crew),
            "fleet" | "implementer" | "worker" => Some(Self::Fleet),
            "review" | "reviewer" => Some(Self::Review),
            "marshal" => Some(Self::Marshal),
            "seneschal" | "concierge" => Some(Self::Seneschal),
            "role" | "beadle" | "gargoyle" | "drawbridge" | "scryer" => Some(Self::Role),
            "any" | "all" | "" => Some(Self::Any),
            _ => None,
        }
    }

    pub fn allows_kind(self, kind: BeadKind) -> bool {
        match self {
            Self::Fleet => matches!(kind, BeadKind::Implement | BeadKind::Task),
            Self::Review => matches!(kind, BeadKind::Review),
            Self::Crew => matches!(
                kind,
                BeadKind::Design | BeadKind::Review | BeadKind::Task
            ),
            Self::Marshal | Self::Seneschal | Self::Role | Self::Any => true,
        }
    }

    /// Infer from seat name when profile caste is Any.
    pub fn infer_from_name(name: &str) -> Self {
        let n = name.to_lowercase();
        if n.starts_with("fleet") || n.starts_with("opus") {
            Self::Fleet
        } else if n.starts_with("crew") || n.starts_with("fable") {
            Self::Crew
        } else if n.starts_with("review") {
            Self::Review
        } else if n.starts_with("marshal") {
            Self::Marshal
        } else if n.starts_with("seneschal") {
            Self::Seneschal
        } else if n.starts_with("beadle")
            || n.starts_with("gargoyle")
            || n.starts_with("drawbridge")
            || n.starts_with("scryer")
        {
            Self::Role
        } else {
            Self::Any
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeatProfile {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub caste: SeatCaste,
    #[serde(default)]
    pub pronouns: String,
    #[serde(default)]
    pub standing_orders: String,
    /// Optional model override for headless workers bound to this seat (fleet).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional provider override (same seat fleet profiles).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub diary: Vec<HandoffNotes>,
    #[serde(default)]
    pub laurels: Vec<Laurel>,
    pub created_at: String,
}

impl SeatProfile {
    pub fn new(name: String) -> Self {
        let caste = SeatCaste::infer_from_name(&name);
        Self {
            name,
            role: String::new(),
            caste,
            pronouns: String::new(),
            standing_orders: String::new(),
            model: None,
            provider: None,
            diary: Vec::new(),
            laurels: Vec::new(),
            created_at: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        }
    }

    pub fn effective_caste(&self) -> SeatCaste {
        if self.caste == SeatCaste::Any {
            SeatCaste::infer_from_name(&self.name)
        } else {
            self.caste
        }
    }

    pub fn slug(&self) -> String {
        slugify(&self.name)
    }

    pub fn wake_identity_block(&self) -> String {
        let mut out = format!("## Seat: {}\n", self.name);
        out.push_str(&format!("Caste: {}\n", self.effective_caste().as_str()));
        if !self.pronouns.trim().is_empty() {
            out.push_str(&format!("Pronouns: {}\n", self.pronouns.trim()));
        }
        if !self.role.trim().is_empty() {
            out.push_str(&format!("Role: {}\n", self.role.trim()));
        }
        if let Some(ref m) = self.model {
            if !m.trim().is_empty() {
                out.push_str(&format!("Preferred model: {}\n", m.trim()));
            }
        }
        if !self.standing_orders.trim().is_empty() {
            out.push_str(&format!(
                "Standing orders:\n{}\n",
                self.standing_orders.trim()
            ));
        }
        out.push_str(
            "You are waking into this named seat. Treat continuity and handoff notes as your diary.\n",
        );
        out
    }

    pub fn append_handoff(&mut self, notes: HandoffNotes) {
        self.diary.push(notes);
        const MAX: usize = 40;
        if self.diary.len() > MAX {
            self.diary = self.diary.split_off(self.diary.len() - MAX);
        }
    }

    pub fn append_laurel(&mut self, laurel: Laurel) {
        self.laurels.push(laurel);
        const MAX: usize = 30;
        if self.laurels.len() > MAX {
            self.laurels = self.laurels.split_off(self.laurels.len() - MAX);
        }
    }

    pub fn last_handoff(&self) -> Option<&HandoffNotes> {
        self.diary.last()
    }
}

/// Resolve effective caste for a claimant/seat name (load profile or infer).
pub fn resolve_caste(name: &str) -> SeatCaste {
    load(name)
        .map(|s| s.effective_caste())
        .unwrap_or_else(|_| SeatCaste::infer_from_name(name))
}

/// Ensure a seat exists and seal its caste (used by fleet up / role runners).
pub fn ensure_with_caste(name: &str, caste: SeatCaste) -> Result<SeatProfile, String> {
    let mut seat = load_or_create(name)?;
    if seat.caste != caste {
        seat.caste = caste;
        save(&seat)?;
    }
    Ok(seat)
}

pub fn slugify(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "seat".into()
    } else {
        trimmed.to_string()
    }
}

fn seats_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".rs-agent").join("seats")
}

pub fn path_for(slug: &str) -> PathBuf {
    seats_dir().join(format!("{slug}.json"))
}

pub fn load(name_or_slug: &str) -> Result<SeatProfile, String> {
    let slug = slugify(name_or_slug);
    let path = path_for(&slug);
    let text = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("No seat `{slug}`. Create with /seat {name_or_slug}")
        } else {
            format!("read seat: {e}")
        }
    })?;
    serde_json::from_str(&text).map_err(|e| format!("parse seat: {e}"))
}

pub fn save(seat: &SeatProfile) -> Result<(), String> {
    let dir = seats_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir seats: {e}"))?;
    let path = path_for(&seat.slug());
    let text = serde_json::to_string_pretty(seat).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write seat: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename seat: {e}"))?;
    Ok(())
}

/// Load or create a seat by name.
pub fn load_or_create(name: &str) -> Result<SeatProfile, String> {
    let slug = slugify(name);
    if path_for(&slug).is_file() {
        load(name)
    } else {
        let seat = SeatProfile::new(name.trim().to_string());
        save(&seat)?;
        Ok(seat)
    }
}

/// Rename a seat, preserving diary/history. Old file removed after save.
pub fn rename(old_name: &str, new_name: &str) -> Result<SeatProfile, String> {
    let mut seat = load(old_name)?;
    let old_slug = seat.slug();
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("new name must not be empty".into());
    }
    let note = HandoffNotes::new(
        format!("Renamed from {} to {new_name}", seat.name),
        String::new(),
        String::new(),
        vec![],
    );
    seat.append_handoff(note);
    seat.name = new_name.to_string();
    save(&seat)?;
    let new_slug = seat.slug();
    if old_slug != new_slug {
        let _ = fs::remove_file(path_for(&old_slug));
    }
    Ok(seat)
}

pub fn list_names() -> Vec<String> {
    let dir = seats_dir();
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(seat) = serde_json::from_str::<SeatProfile>(&text) {
                names.push(seat.name);
            }
        }
    }
    names.sort();
    names
}

/// Parse `/seat` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatCommand {
    Status,
    Clear,
    List,
    Bind(String),
    SetPronouns(String),
    SetRole(String),
    SetOrders(String),
    SetModel(Option<String>),
    SetCaste(SeatCaste),
    Rename(String),
}

pub fn parse_seat_arg(arg: &str) -> Result<SeatCommand, String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Ok(SeatCommand::Status);
    }
    let lower = arg.to_lowercase();
    match lower.as_str() {
        "clear" | "off" | "none" | "unbind" => Ok(SeatCommand::Clear),
        "list" | "ls" => Ok(SeatCommand::List),
        _ => {
            let mut parts = arg.splitn(2, char::is_whitespace);
            let cmd = parts.next().unwrap_or("").to_lowercase();
            let rest = parts.next().unwrap_or("").trim();
            match cmd.as_str() {
                "pronouns" | "pronoun" => {
                    if rest.is_empty() {
                        Err("Usage: /seat pronouns she/her".into())
                    } else {
                        Ok(SeatCommand::SetPronouns(rest.into()))
                    }
                }
                "role" => {
                    if rest.is_empty() {
                        Err("Usage: /seat role <description>".into())
                    } else {
                        Ok(SeatCommand::SetRole(rest.into()))
                    }
                }
                "caste" => {
                    if rest.is_empty() {
                        Err(
                            "Usage: /seat caste crew|fleet|review|marshal|seneschal|role|any"
                                .into(),
                        )
                    } else {
                        SeatCaste::parse(rest)
                            .map(SeatCommand::SetCaste)
                            .ok_or_else(|| format!("unknown caste `{rest}`"))
                    }
                }
                "orders" | "standing" => {
                    if rest.is_empty() {
                        Err("Usage: /seat orders <text>".into())
                    } else {
                        Ok(SeatCommand::SetOrders(rest.into()))
                    }
                }
                "model" => {
                    if rest.is_empty() || rest.eq_ignore_ascii_case("clear") {
                        Ok(SeatCommand::SetModel(None))
                    } else {
                        Ok(SeatCommand::SetModel(Some(rest.into())))
                    }
                }
                "rename" => {
                    if rest.is_empty() {
                        Err("Usage: /seat rename <new-name>".into())
                    } else {
                        Ok(SeatCommand::Rename(rest.into()))
                    }
                }
                _ => Ok(SeatCommand::Bind(arg.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Fox"), "fox");
        assert_eq!(slugify("Sea Gull"), "sea-gull");
    }

    #[test]
    fn parse_seat_commands() {
        assert_eq!(parse_seat_arg("").unwrap(), SeatCommand::Status);
        assert_eq!(parse_seat_arg("clear").unwrap(), SeatCommand::Clear);
        assert_eq!(
            parse_seat_arg("Fox").unwrap(),
            SeatCommand::Bind("Fox".into())
        );
        assert_eq!(
            parse_seat_arg("pronouns she/her").unwrap(),
            SeatCommand::SetPronouns("she/her".into())
        );
    }

    #[test]
    fn rename_preserves_diary() {
        let tmp = tempfile::tempdir().unwrap();
        let mut seat = SeatProfile::new("Spider".into());
        seat.append_handoff(HandoffNotes::new(
            "did stuff".into(),
            "".into(),
            "".into(),
            vec![],
        ));
        seat.name = "Lark".into();
        seat.append_handoff(HandoffNotes::new(
            "Renamed from Spider to Lark".into(),
            "".into(),
            "".into(),
            vec![],
        ));
        assert_eq!(seat.diary.len(), 2);
        assert!(seat.diary[1].summary.contains("Renamed"));
        let _ = tmp;
    }

    #[test]
    fn parse_caste_command() {
        assert_eq!(
            parse_seat_arg("caste fleet").unwrap(),
            SeatCommand::SetCaste(SeatCaste::Fleet)
        );
        assert!(SeatCaste::Fleet.allows_kind(BeadKind::Implement));
        assert!(!SeatCaste::Fleet.allows_kind(BeadKind::Design));
        assert!(SeatCaste::Crew.allows_kind(BeadKind::Design));
    }
}
