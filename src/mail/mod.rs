//! City mail — inbox/outbox under `.rs-agent/mail/`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub bead_refs: Vec<String>,
    pub created_at: String,
    #[serde(default)]
    pub acked: bool,
    #[serde(default)]
    pub acked_at: Option<String>,
}

fn project_rs_agent() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
}

pub fn mail_dir() -> PathBuf {
    project_rs_agent().join("mail")
}

pub fn inbox_dir() -> PathBuf {
    mail_dir().join("inbox")
}

pub fn outbox_dir() -> PathBuf {
    mail_dir().join("outbox")
}

fn ensure_dirs() -> Result<(), String> {
    fs::create_dir_all(inbox_dir()).map_err(|e| format!("mkdir mail inbox: {e}"))?;
    fs::create_dir_all(outbox_dir()).map_err(|e| format!("mkdir mail outbox: {e}"))?;
    Ok(())
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Send mail to a seat (or `broadcast` / `human` / `Seneschal`).
pub fn send(
    from: &str,
    to: &str,
    body: &str,
    bead_refs: Vec<String>,
) -> Result<MailMessage, String> {
    ensure_dirs()?;
    let body = body.trim();
    if body.is_empty() {
        return Err("mail body must not be empty".into());
    }
    let to = to.trim();
    if to.is_empty() {
        return Err("mail to must not be empty".into());
    }
    let msg = MailMessage {
        id: format!("m{}", &Uuid::new_v4().to_string()[..8]),
        from: from.trim().to_string(),
        to: to.to_string(),
        body: body.to_string(),
        bead_refs,
        created_at: now_str(),
        acked: false,
        acked_at: None,
    };
    let path = inbox_dir().join(format!("{}.json", msg.id));
    let text = serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("write mail: {e}"))?;
    // Mirror to outbox for the sender.
    let out = outbox_dir().join(format!("{}.json", msg.id));
    let _ = fs::write(&out, serde_json::to_string_pretty(&msg).unwrap_or_default());
    Ok(msg)
}

fn read_msg(path: &Path) -> Option<MailMessage> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn list_dir(dir: &Path, include_acked: bool) -> Vec<MailMessage> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<MailMessage> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                return None;
            }
            read_msg(&p)
        })
        .filter(|m| include_acked || !m.acked)
        .collect();
    items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    items
}

pub fn inbox(for_seat: Option<&str>, include_acked: bool) -> Vec<MailMessage> {
    let all = list_dir(&inbox_dir(), include_acked);
    match for_seat {
        None => all,
        Some(seat) => {
            let seat_l = seat.to_lowercase();
            all.into_iter()
                .filter(|m| {
                    let t = m.to.to_lowercase();
                    t == "broadcast"
                        || t == "human"
                        || t == seat_l
                        || (seat_l.contains("seneschal") && t == "seneschal")
                })
                .collect()
        }
    }
}

pub fn ack(id: &str) -> Result<MailMessage, String> {
    ensure_dirs()?;
    let id = id.trim().trim_start_matches('@');
    let path = inbox_dir().join(format!("{id}.json"));
    if !path.exists() {
        return Err(format!("mail `{id}` not found"));
    }
    let mut msg = read_msg(&path).ok_or_else(|| format!("mail `{id}` corrupt"))?;
    msg.acked = true;
    msg.acked_at = Some(now_str());
    let text = serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("write mail: {e}"))?;
    Ok(msg)
}

pub fn format_inbox(for_seat: Option<&str>) -> String {
    let items = inbox(for_seat, false);
    if items.is_empty() {
        return "Mail inbox empty.".into();
    }
    let mut out = format!("Mail inbox ({} unacked):\n", items.len());
    for m in items {
        let refs = if m.bead_refs.is_empty() {
            String::new()
        } else {
            format!(" beads:{}", m.bead_refs.join(","))
        };
        out.push_str(&format!(
            "  {}  {} → {}  [{}]{}  {}\n",
            m.id,
            m.from,
            m.to,
            m.created_at,
            refs,
            truncate(&m.body, 80)
        ));
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_ack_roundtrip() {
        crate::with_temp_cwd(|_| {
            let msg = send(
                "Fleet-1",
                "Seneschal",
                "stuck on secrets",
                vec!["b1".into()],
            )
            .unwrap();
            assert!(!msg.acked);
            let listed = inbox(Some("Seneschal"), false);
            assert_eq!(listed.len(), 1);
            ack(&msg.id).unwrap();
            assert!(inbox(Some("Seneschal"), false).is_empty());
        });
    }
}
