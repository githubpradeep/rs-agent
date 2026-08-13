//! Prompt templates: small markdown + frontmatter snippets with an `{{args}}`
//! placeholder, invocable as e.g. `/prompt fix "the null pointer in parser.rs"`.

use std::path::{Path, PathBuf};

/// A single discovered prompt template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub body: String,
    pub path: PathBuf,
}

const RS_AGENT_DIR: &str = ".rs-agent";
const PROMPTS_DIR: &str = "prompts";
const ARGS_PLACEHOLDER: &str = "{{args}}";

fn home_config_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(RS_AGENT_DIR)
}

/// Directories to search for templates, in override order (later wins on name
/// clash): `~/.rs-agent/prompts/`, then `./.rs-agent/prompts/`.
fn template_search_dirs() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    vec![
        home_config_dir().join(PROMPTS_DIR),
        cwd.join(RS_AGENT_DIR).join(PROMPTS_DIR),
    ]
}

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
struct TemplateFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (None, content);
    };
    let rest = match rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
    {
        Some(r) => r,
        None => return (None, content),
    };
    let Some(end_idx) = rest.find("\n---") else {
        return (None, content);
    };
    let yaml = &rest[..end_idx];
    let after_delim = &rest[end_idx + 4..];
    let body = after_delim
        .strip_prefix('\n')
        .or_else(|| after_delim.strip_prefix("\r\n"))
        .unwrap_or(after_delim);
    (Some(yaml), body)
}

fn parse_template(path: &Path, content: &str) -> Template {
    let (yaml, body) = split_frontmatter(content);
    let frontmatter: TemplateFrontmatter = yaml
        .and_then(|y| serde_yaml::from_str(y).ok())
        .unwrap_or_default();

    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("template")
        .to_string();

    let name = frontmatter
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(file_stem);
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

    Template {
        name,
        description,
        body: body_trimmed,
        path: path.to_path_buf(),
    }
}

/// Discover all prompt templates from the standard search paths.
///
/// Later directories override earlier ones when two templates share a `name`.
/// Search order: `~/.rs-agent/prompts/`, then `./.rs-agent/prompts/`.
pub fn discover_templates() -> Vec<Template> {
    let mut by_name: std::collections::BTreeMap<String, Template> =
        std::collections::BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    for dir in template_search_dirs() {
        for file in find_markdown_files(&dir) {
            let content = match std::fs::read_to_string(&file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let template = parse_template(&file, &content);
            if !by_name.contains_key(&template.name) {
                order.push(template.name.clone());
            }
            by_name.insert(template.name.clone(), template);
        }
    }

    order
        .into_iter()
        .filter_map(|name| by_name.remove(&name))
        .collect()
}

/// Find a template by exact name (case-insensitive).
pub fn find_template<'a>(templates: &'a [Template], name: &str) -> Option<&'a Template> {
    templates.iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// Render a template by substituting all `{{args}}` occurrences with `args`.
/// If the template has no placeholder, `args` is appended on a new line
/// (when non-empty) so invocation input is never silently dropped.
pub fn render_template(t: &Template, args: &str) -> String {
    if t.body.contains(ARGS_PLACEHOLDER) {
        t.body.replace(ARGS_PLACEHOLDER, args)
    } else if args.trim().is_empty() {
        t.body.clone()
    } else {
        format!("{}\n\n{}", t.body, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_name() {
        let content = "---\nname: fix\n---\nPlease fix: {{args}}\n";
        let t = parse_template(Path::new("/tmp/fix.md"), content);
        assert_eq!(t.name, "fix");
        assert_eq!(t.body.trim_end(), "Please fix: {{args}}");
    }

    #[test]
    fn falls_back_to_filename_without_frontmatter() {
        let content = "Please explain: {{args}}\n";
        let t = parse_template(Path::new("/tmp/explain.md"), content);
        assert_eq!(t.name, "explain");
        assert_eq!(t.description, "Please explain: {{args}}");
    }

    #[test]
    fn render_template_replaces_all_occurrences() {
        let t = Template {
            name: "fix".to_string(),
            description: String::new(),
            body: "Please fix: {{args}}\nRemember: {{args}} matters.".to_string(),
            path: PathBuf::from("/tmp/fix.md"),
        };
        let rendered = render_template(&t, "the null pointer bug");
        assert_eq!(
            rendered,
            "Please fix: the null pointer bug\nRemember: the null pointer bug matters."
        );
    }

    #[test]
    fn render_template_without_placeholder_appends_args() {
        let t = Template {
            name: "review".to_string(),
            description: String::new(),
            body: "Review the current diff for bugs.".to_string(),
            path: PathBuf::from("/tmp/review.md"),
        };
        assert_eq!(
            render_template(&t, "focus on security"),
            "Review the current diff for bugs.\n\nfocus on security"
        );
        assert_eq!(render_template(&t, ""), "Review the current diff for bugs.");
        assert_eq!(
            render_template(&t, "   "),
            "Review the current diff for bugs."
        );
    }

    #[test]
    fn find_template_is_case_insensitive() {
        let templates = vec![Template {
            name: "Fix".to_string(),
            description: String::new(),
            body: "{{args}}".to_string(),
            path: PathBuf::from("/tmp/fix.md"),
        }];
        assert!(find_template(&templates, "fix").is_some());
        assert!(find_template(&templates, "FIX").is_some());
        assert!(find_template(&templates, "nope").is_none());
    }
}
