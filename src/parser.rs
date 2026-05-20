//! Markdown parsing: frontmatter, body, wikilink extraction.

use anyhow::{Context, Result};
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrontmatterMetadata {
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<FrontmatterMetadata>,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub slug: String,
    pub title: String,
    pub source: String,
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body_html: String,
    pub outgoing_links: Vec<String>,
}

impl Note {
    pub fn note_type(&self) -> &str {
        self.frontmatter
            .metadata
            .as_ref()
            .and_then(|m| m.note_type.as_deref())
            .unwrap_or("note")
    }

    pub fn status(&self) -> Option<&str> {
        self.frontmatter
            .metadata
            .as_ref()
            .and_then(|m| m.status.as_deref())
    }

    pub fn description(&self) -> &str {
        self.frontmatter
            .description
            .as_deref()
            .unwrap_or("")
    }
}

/// Parse a single markdown file into a Note.
pub fn parse_file(path: &Path, source_name: &str) -> Result<Note> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let (frontmatter, body_md) = split_frontmatter(&raw);
    let slug = derive_slug(&frontmatter, path);
    let title = derive_title(&frontmatter, &body_md, &slug);

    // Replace [[wikilink]] with markdown links AND capture targets.
    let mut outgoing = Vec::new();
    let processed_md = WIKILINK_RE
        .get_or_init(|| Regex::new(r"\[\[([^\]\|]+)(?:\|([^\]]+))?\]\]").unwrap())
        .replace_all(&body_md, |caps: &regex::Captures| {
            let target = caps.get(1).unwrap().as_str().trim().to_string();
            let display = caps.get(2).map(|m| m.as_str().trim()).unwrap_or(&target);
            let slug = slugify(&target);
            outgoing.push(slug.clone());
            format!("[{}](./{}.html)", display, slug)
        })
        .into_owned();

    // Render markdown to HTML.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(&processed_md, options);
    let mut body_html = String::new();
    html::push_html(&mut body_html, parser);

    outgoing.sort();
    outgoing.dedup();

    Ok(Note {
        slug,
        title,
        source: source_name.to_string(),
        path: path.to_path_buf(),
        frontmatter,
        body_html,
        outgoing_links: outgoing,
    })
}

static WIKILINK_RE: OnceLock<Regex> = OnceLock::new();
static H1_RE: OnceLock<Regex> = OnceLock::new();

fn split_frontmatter(raw: &str) -> (Frontmatter, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (Frontmatter::default(), raw.to_string());
    }
    // Find the closing --- after the opening one.
    let after_open = &trimmed[3..];
    if let Some(end_offset) = after_open.find("\n---") {
        let yaml = after_open[..end_offset].trim();
        let rest = &after_open[end_offset + 4..]; // skip "\n---"
        let body = rest.trim_start_matches('\n').to_string();
        let frontmatter: Frontmatter = serde_yaml::from_str(yaml).unwrap_or_default();
        (frontmatter, body)
    } else {
        (Frontmatter::default(), raw.to_string())
    }
}

fn derive_slug(frontmatter: &Frontmatter, path: &Path) -> String {
    if let Some(name) = frontmatter.name.as_deref() {
        return slugify(name);
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");
    slugify(stem)
}

fn derive_title(frontmatter: &Frontmatter, body_md: &str, slug: &str) -> String {
    // Prefer first H1 in the body.
    let re = H1_RE.get_or_init(|| Regex::new(r"(?m)^#\s+(.+)$").unwrap());
    if let Some(caps) = re.captures(body_md) {
        return caps.get(1).unwrap().as_str().trim().to_string();
    }
    // Fallback: frontmatter name or slug
    frontmatter
        .name
        .clone()
        .unwrap_or_else(|| slug.replace('-', " "))
}

/// Convert "Some Title" / "snake_case_name" / "kebab-name" → "kebab-name".
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn slugify_handles_punctuation() {
        assert_eq!(slugify("Project Statistics & Reasoning!"), "project-statistics-reasoning");
        assert_eq!(slugify("snake_case_name"), "snake-case-name");
        assert_eq!(slugify("already-kebab"), "already-kebab");
    }

    #[test]
    fn frontmatter_round_trips() {
        let raw = "---\nname: foo-bar\ndescription: a thing\nmetadata:\n  type: project\n---\n\n# Foo\n\nBody";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.name.as_deref(), Some("foo-bar"));
        assert_eq!(fm.description.as_deref(), Some("a thing"));
        assert_eq!(fm.metadata.unwrap().note_type.as_deref(), Some("project"));
        assert!(body.starts_with("# Foo"));
    }

    #[test]
    fn parse_extracts_wikilinks() {
        let f = write_temp("---\nname: a\n---\n\nlinks to [[other-note]] and [[Display|alias-target]]");
        let note = parse_file(f.path(), "test").unwrap();
        assert!(note.outgoing_links.contains(&"other-note".to_string()));
        assert!(note.outgoing_links.contains(&"alias-target".to_string()));
    }
}
