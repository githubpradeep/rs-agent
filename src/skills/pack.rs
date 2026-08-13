//! Skill pack export/import — zip + manifest for sharing skills.

use crate::skills::{discover_skills, Skill};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPackManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub skills: Vec<SkillPackEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPackEntry {
    pub file: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Export named skills (or all if `names` empty) to `out_path` (.zip).
pub fn export_pack(names: &[String], out_path: &Path) -> Result<String, String> {
    let skills = discover_skills();
    let selected: Vec<&Skill> = if names.is_empty() {
        skills.iter().collect()
    } else {
        names
            .iter()
            .filter_map(|n| skills.iter().find(|s| s.name.eq_ignore_ascii_case(n)))
            .collect()
    };
    if selected.is_empty() {
        return Err("No skills to export (check names with /skills)".into());
    }

    let file = File::create(out_path).map_err(|e| format!("create {}: {e}", out_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut entries = Vec::new();
    for skill in &selected {
        let file_name = format!(
            "{}.md",
            skill
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(skill.name.as_str())
        );
        let content = fs::read_to_string(&skill.path)
            .map_err(|e| format!("read {}: {e}", skill.path.display()))?;
        zip.start_file(&file_name, opts)
            .map_err(|e| format!("zip start: {e}"))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| format!("zip write: {e}"))?;
        entries.push(SkillPackEntry {
            file: file_name,
            name: Some(skill.name.clone()),
        });
    }

    let pack_name = out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skills")
        .to_string();
    let manifest = SkillPackManifest {
        name: pack_name,
        version: "1".into(),
        description: format!("{} skill(s)", entries.len()),
        skills: entries,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("manifest: {e}"))?;
    zip.start_file("manifest.json", opts)
        .map_err(|e| format!("zip manifest: {e}"))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(|e| format!("zip write: {e}"))?;
    zip.finish().map_err(|e| format!("zip finish: {e}"))?;

    Ok(format!(
        "Exported {} skill(s) to {}",
        selected.len(),
        out_path.display()
    ))
}

/// Import a skill pack zip into `~/.rs-agent/skills/<pack>/`.
pub fn import_pack(zip_path: &Path) -> Result<String, String> {
    if !zip_path.is_file() {
        return Err(format!("Not a file: {}", zip_path.display()));
    }
    let file = File::open(zip_path).map_err(|e| format!("open: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("zip open: {e}"))?;

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    let pack_stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let dest = PathBuf::from(home)
        .join(".rs-agent")
        .join("skills")
        .join(pack_stem);
    fs::create_dir_all(&dest).map_err(|e| format!("mkdir: {e}"))?;

    let mut imported = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry: {e}"))?;
        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let base = Path::new(&name)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(name);
        // Only markdown + manifest
        if !(base.ends_with(".md") || base == "manifest.json") {
            continue;
        }
        let out_path = dest.join(&base);
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("read entry: {e}"))?;
        fs::write(&out_path, &buf).map_err(|e| format!("write {}: {e}", out_path.display()))?;
        if base.ends_with(".md") {
            imported += 1;
        }
    }

    Ok(format!(
        "Imported {imported} skill file(s) into {}",
        dest.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_empty_names_errors_without_skills_ok() {
        // Just ensure export with impossible name fails cleanly.
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("pack.zip");
        let err = export_pack(&["___no_such_skill___".into()], &out).unwrap_err();
        assert!(err.contains("No skills"));
    }
}
