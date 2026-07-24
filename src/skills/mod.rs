//! Skills v1: markdown + YAML-frontmatter instruction packs discovered from disk.
//!
//! A skill is a reusable chunk of model instructions (a "how to do X" recipe)
//! that can be injected into the system/user context on demand (e.g. via a
//! future `/skill <name>` command). See [`docs/skills.md`] for the authoring
//! guide.

pub mod templates;

pub use templates::{discover_templates, find_template, render_template, Template};

use std::path::{Path, PathBuf};

/// A single discovered skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub body: String,
    pub path: PathBuf,
}

const RS_AGENT_DIR: &str = ".rs-agent";
const SKILLS_DIR: &str = "skills";

/// `~/.rs-agent`
fn home_config_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(RS_AGENT_DIR)
}

/// Directories to search for skills, in override order (later wins on name clash):
/// 1. `~/.rs-agent/skills/`
/// 2. `./.rs-agent/skills/` (project-local, hidden)
/// 3. `./skills/` (project-shared, checked into the repo)
fn skills_search_dirs() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    vec![
        home_config_dir().join(SKILLS_DIR),
        cwd.join(RS_AGENT_DIR).join(SKILLS_DIR),
        cwd.join(SKILLS_DIR),
    ]
}

/// Recursively find `*.md` files under `dir`, sorted for deterministic ordering.
fn find_markdown_files(dir: &Path) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "md"))
        .collect();
    files.sort();
    files
}

#[derive(Debug, Default, serde::Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    triggers: Vec<String>,
}

/// Split a markdown file into `(Some(frontmatter_yaml), body)` if it starts with
/// a `---` delimited YAML block, otherwise `(None, whole_content)`.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (None, content);
    };
    // Frontmatter delimiter must be on its own line.
    let rest = match rest.strip_prefix('\n').or_else(|| rest.strip_prefix("\r\n")) {
        Some(r) => r,
        None => return (None, content),
    };
    let Some(end_idx) = rest.find("\n---") else {
        return (None, content);
    };
    let yaml = &rest[..end_idx];
    // Skip past the closing `---` line (and its trailing newline, if any).
    let after_delim = &rest[end_idx + 4..];
    let body = after_delim
        .strip_prefix('\n')
        .or_else(|| after_delim.strip_prefix("\r\n"))
        .unwrap_or(after_delim);
    (Some(yaml), body)
}

/// Parse a skill markdown file's contents into a [`Skill`].
///
/// If there's no frontmatter, `name` falls back to the file stem and
/// `description` falls back to the first non-empty line of the body.
fn parse_skill(path: &Path, content: &str) -> Skill {
    let (yaml, body) = split_frontmatter(content);
    let frontmatter: SkillFrontmatter = yaml
        .and_then(|y| serde_yaml::from_str(y).ok())
        .unwrap_or_default();

    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();

    let name = frontmatter.name.filter(|s| !s.trim().is_empty()).unwrap_or(file_stem);

    let body_trimmed = body.trim_start_matches(['\n', '\r']).to_string();

    let description = frontmatter
        .description
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            body_trimmed
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string()
        });

    Skill {
        name,
        description,
        triggers: frontmatter.triggers,
        body: body_trimmed,
        path: path.to_path_buf(),
    }
}

/// Discover all skills from the standard search paths.
///
/// Later directories override earlier ones when two skills share a `name`.
/// Search order: `~/.rs-agent/skills/`, `./.rs-agent/skills/`, `./skills/`.
pub fn discover_skills() -> Vec<Skill> {
    let mut by_name: std::collections::BTreeMap<String, Skill> = std::collections::BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    for dir in skills_search_dirs() {
        for file in find_markdown_files(&dir) {
            let content = match std::fs::read_to_string(&file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let skill = parse_skill(&file, &content);
            if !by_name.contains_key(&skill.name) {
                order.push(skill.name.clone());
            }
            by_name.insert(skill.name.clone(), skill);
        }
    }

    order
        .into_iter()
        .filter_map(|name| by_name.remove(&name))
        .collect()
}

/// Find a skill by exact name (case-insensitive).
pub fn find_skill<'a>(skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    skills.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Format a skill as an XML-ish block suitable for appending to a system or
/// user message so the model receives the skill's instructions verbatim.
pub fn format_skill_injection(skill: &Skill) -> String {
    format!(
        "<skill name=\"{}\" description=\"{}\">\n{}\n</skill>",
        skill.name, skill.description, skill.body
    )
}

/// Render a short, human-readable summary list of all discovered skills,
/// e.g. for a `/skills` listing command.
pub fn list_skills_summary(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return "No skills found.".to_string();
    }
    let mut out = String::new();
    for skill in skills {
        out.push_str(&format!("- {}: {}", skill.name, skill.description));
        if !skill.triggers.is_empty() {
            out.push_str(&format!(" (triggers: {})", skill.triggers.join(", ")));
        }
        out.push('\n');
    }
    out.pop();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_fields() {
        let content = r#"---
name: pr-review
description: Review a pull request for bugs and style
triggers: ["pr", "pull request", "review"]
---
Skill body instructions for the model...

Second paragraph.
"#;
        let skill = parse_skill(Path::new("/tmp/pr-review.md"), content);
        assert_eq!(skill.name, "pr-review");
        assert_eq!(skill.description, "Review a pull request for bugs and style");
        assert_eq!(skill.triggers, vec!["pr", "pull request", "review"]);
        assert!(skill.body.starts_with("Skill body instructions for the model..."));
        assert!(skill.body.contains("Second paragraph."));
    }

    #[test]
    fn falls_back_to_filename_and_first_line_without_frontmatter() {
        let content = "\nFix the failing test.\n\nMore details here.\n";
        let skill = parse_skill(Path::new("/tmp/fix-tests.md"), content);
        assert_eq!(skill.name, "fix-tests");
        assert_eq!(skill.description, "Fix the failing test.");
        assert!(skill.triggers.is_empty());
        assert_eq!(skill.body.trim_start(), skill.body);
    }

    #[test]
    fn partial_frontmatter_falls_back_for_missing_fields() {
        let content = r#"---
name: my-skill
---
First real line.
"#;
        let skill = parse_skill(Path::new("/tmp/whatever.md"), content);
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "First real line.");
    }

    #[test]
    fn no_closing_delimiter_treats_whole_file_as_body() {
        let content = "---\nname: broken\nthis never closes\n";
        let skill = parse_skill(Path::new("/tmp/broken.md"), content);
        // No closing `---`, so the whole thing is treated as plain content.
        assert_eq!(skill.name, "broken");
        assert!(skill.body.contains("this never closes"));
    }

    #[test]
    fn find_skill_is_case_insensitive() {
        let skills = vec![Skill {
            name: "PR-Review".to_string(),
            description: "desc".to_string(),
            triggers: vec![],
            body: "body".to_string(),
            path: PathBuf::from("/tmp/x.md"),
        }];
        assert!(find_skill(&skills, "pr-review").is_some());
        assert!(find_skill(&skills, "PR-REVIEW").is_some());
        assert!(find_skill(&skills, "nope").is_none());
    }

    #[test]
    fn format_skill_injection_wraps_in_tags() {
        let skill = Skill {
            name: "debug".to_string(),
            description: "Debug an issue".to_string(),
            triggers: vec![],
            body: "Reproduce, isolate, fix.".to_string(),
            path: PathBuf::from("/tmp/debug.md"),
        };
        let injected = format_skill_injection(&skill);
        assert!(injected.starts_with("<skill name=\"debug\" description=\"Debug an issue\">"));
        assert!(injected.contains("Reproduce, isolate, fix."));
        assert!(injected.trim_end().ends_with("</skill>"));
    }

    #[test]
    fn list_skills_summary_lists_all_with_triggers() {
        let skills = vec![
            Skill {
                name: "a".to_string(),
                description: "does a".to_string(),
                triggers: vec!["alpha".to_string()],
                body: String::new(),
                path: PathBuf::from("/tmp/a.md"),
            },
            Skill {
                name: "b".to_string(),
                description: "does b".to_string(),
                triggers: vec![],
                body: String::new(),
                path: PathBuf::from("/tmp/b.md"),
            },
        ];
        let summary = list_skills_summary(&skills);
        assert!(summary.contains("- a: does a (triggers: alpha)"));
        assert!(summary.contains("- b: does b"));
        assert!(!summary.contains("b (triggers"));
    }

    #[test]
    fn list_skills_summary_empty() {
        assert_eq!(list_skills_summary(&[]), "No skills found.");
    }

    #[test]
    fn discovery_overrides_by_name_in_search_order() {
        let tmp = tempfile::tempdir().unwrap();
        let home_skills = tmp.path().join("home/.rs-agent/skills");
        let project_skills = tmp.path().join("project/skills");
        std::fs::create_dir_all(&home_skills).unwrap();
        std::fs::create_dir_all(&project_skills).unwrap();

        std::fs::write(
            home_skills.join("shared.md"),
            "---\nname: shared\ndescription: from home\n---\nHome body\n",
        )
        .unwrap();
        std::fs::write(
            project_skills.join("shared.md"),
            "---\nname: shared\ndescription: from project\n---\nProject body\n",
        )
        .unwrap();
        std::fs::write(
            home_skills.join("only-home.md"),
            "---\nname: only-home\ndescription: only in home\n---\nBody\n",
        )
        .unwrap();

        let old_home = std::env::var("HOME").ok();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_var("HOME", tmp.path().join("home"));
        std::env::set_current_dir(tmp.path().join("project")).unwrap();

        let skills = discover_skills();

        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        std::env::set_current_dir(old_cwd).unwrap();

        let shared = find_skill(&skills, "shared").expect("shared skill found");
        assert_eq!(shared.description, "from project");
        assert!(find_skill(&skills, "only-home").is_some());
    }
}
