//! Render notes + index + graph into HTML files.

use crate::graph::{build_backlinks, build_graph, project_subgraph};
use crate::parser::Note;
use crate::{CodeIndex, IndexedChunk};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tera::Tera;

const NOTE_TPL: &str = include_str!("../templates/note.html");
const INDEX_TPL: &str = include_str!("../templates/index.html");
const GRAPH_TPL: &str = include_str!("../templates/graph.html");
const CODE_CHUNK_TPL: &str = include_str!("../templates/code_chunk.html");
const CODE_INDEX_TPL: &str = include_str!("../templates/code_index.html");
const CSS: &str = include_str!("../assets/style.css");
const D3_JS: &str = include_str!("../assets/d3.v7.min.js");

const CATEGORY_ORDER: &[&str] = &["project", "reference", "feedback", "user", "note", "dangling"];

#[derive(Serialize)]
struct NoteSummary {
    slug: String,
    title: String,
    description: String,
    status: String,
}

#[derive(Serialize)]
struct CategoryGroup {
    category: String,
    notes: Vec<NoteSummary>,
}

#[derive(Serialize)]
struct LinkRef {
    slug: String,
    title: String,
    dangling: bool,
}

#[derive(Serialize)]
struct NoteContext<'a> {
    slug: &'a str,
    title: &'a str,
    source: &'a str,
    description: &'a str,
    note_type: &'a str,
    status: &'a str,
    body_html: &'a str,
}

pub fn render_site(
    notes: &[Note],
    code_index: Option<&CodeIndex>,
    site_title: &str,
    output_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(output_dir).context("creating output dir")?;
    fs::create_dir_all(output_dir.join("notes")).context("creating notes dir")?;

    // Write static assets (vendored so the site works fully offline).
    fs::write(output_dir.join("style.css"), CSS).context("writing style.css")?;
    fs::write(output_dir.join("d3.v7.min.js"), D3_JS).context("writing d3.v7.min.js")?;

    // Init Tera with embedded templates.
    let mut tera = Tera::default();
    tera.add_raw_template("note.html", NOTE_TPL)?;
    tera.add_raw_template("index.html", INDEX_TPL)?;
    tera.add_raw_template("graph.html", GRAPH_TPL)?;
    tera.add_raw_template("code_chunk.html", CODE_CHUNK_TPL)?;
    tera.add_raw_template("code_index.html", CODE_INDEX_TPL)?;

    // Categorise notes for sidebar + index.
    let by_type = categorise(notes);

    // Backlinks.
    let backlinks_map = build_backlinks(notes);
    let title_by_slug: BTreeMap<&str, &str> =
        notes.iter().map(|n| (n.slug.as_str(), n.title.as_str())).collect();
    let known_slugs: std::collections::HashSet<&str> =
        notes.iter().map(|n| n.slug.as_str()).collect();

    // Per-note pages.
    for note in notes {
        let mut ctx = tera::Context::new();
        ctx.insert("site_title", site_title);
        ctx.insert(
            "note",
            &NoteContext {
                slug: &note.slug,
                title: &note.title,
                source: &note.source,
                description: note.description(),
                note_type: note.note_type(),
                status: note.status().unwrap_or(""),
                body_html: &note.body_html,
            },
        );

        // Backlinks for this note (incoming).
        let backlinks: Vec<LinkRef> = backlinks_map
            .get(&note.slug)
            .map(|v| {
                v.iter()
                    .map(|n| LinkRef {
                        slug: n.slug.clone(),
                        title: n.title.clone(),
                        dangling: false,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Outgoing links — resolve titles or mark dangling.
        let outgoing: Vec<LinkRef> = note
            .outgoing_links
            .iter()
            .map(|slug| {
                if let Some(title) = title_by_slug.get(slug.as_str()) {
                    LinkRef {
                        slug: slug.clone(),
                        title: title.to_string(),
                        dangling: !known_slugs.contains(slug.as_str()),
                    }
                } else {
                    LinkRef {
                        slug: slug.clone(),
                        title: slug.replace('-', " "),
                        dangling: true,
                    }
                }
            })
            .collect();

        ctx.insert("backlinks", &backlinks);
        ctx.insert("outgoing", &outgoing);
        ctx.insert("by_type", &by_type);

        let rendered = tera
            .render("note.html", &ctx)
            .with_context(|| format!("rendering {}", note.slug))?;
        let out_path = output_dir.join("notes").join(format!("{}.html", note.slug));
        fs::write(&out_path, rendered).with_context(|| format!("writing {}", out_path.display()))?;
    }

    // Index page.
    let mut ctx = tera::Context::new();
    ctx.insert("site_title", site_title);
    ctx.insert("n_notes", &notes.len());
    ctx.insert("by_type", &by_type);
    let rendered = tera.render("index.html", &ctx)?;
    fs::write(output_dir.join("index.html"), rendered)?;

    // Graph page.
    let full_graph = build_graph(notes, code_index);
    let project_graph = project_subgraph(&full_graph);
    let mut ctx = tera::Context::new();
    ctx.insert("site_title", site_title);
    ctx.insert("graph_full_json", &serde_json::to_string(&full_graph)?);
    ctx.insert(
        "graph_projects_json",
        &serde_json::to_string(&project_graph)?,
    );
    let rendered = tera.render("graph.html", &ctx)?;
    fs::write(output_dir.join("graph.html"), rendered)?;

    // Code chunk pages + index (only if a code index exists).
    if let Some(ci) = code_index {
        render_code_pages(ci, site_title, output_dir, &tera)?;
    }

    // llms.txt — AI crawl manifest (includes code chunks when available).
    write_llms_txt(notes, code_index, site_title, output_dir)?;

    Ok(())
}

// ---------- Code chunk rendering ----------

#[derive(Serialize)]
struct SiblingInfo {
    encoded_id: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Serialize)]
struct CodeChunkSummary {
    encoded_id: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Serialize)]
struct CodeFileGroup {
    path: String,
    chunks: Vec<CodeChunkSummary>,
}

#[derive(Serialize)]
struct CodeProjectGroup {
    name: String,
    files: Vec<CodeFileGroup>,
    n_chunks: usize,
}

fn render_code_pages(
    ci: &CodeIndex,
    site_title: &str,
    output_dir: &Path,
    tera: &Tera,
) -> Result<()> {
    let code_dir = output_dir.join("code");
    fs::create_dir_all(&code_dir).context("creating output/code dir")?;

    // Group siblings by (project, path).
    let mut by_file: BTreeMap<(String, String), Vec<&IndexedChunk>> = BTreeMap::new();
    for chunk in &ci.chunks {
        by_file
            .entry((chunk.chunk.project.clone(), chunk.chunk.path.clone()))
            .or_default()
            .push(chunk);
    }
    for v in by_file.values_mut() {
        v.sort_by_key(|c| c.chunk.start_line);
    }

    // Per-chunk page.
    for chunk in &ci.chunks {
        let key = (chunk.chunk.project.clone(), chunk.chunk.path.clone());
        let siblings: Vec<SiblingInfo> = by_file
            .get(&key)
            .map(|sibs| {
                sibs.iter()
                    .filter(|s| s.chunk.id != chunk.chunk.id)
                    .map(|s| SiblingInfo {
                        encoded_id: encode_id(&s.chunk.id),
                        start_line: s.chunk.start_line,
                        end_line: s.chunk.end_line,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let language = language_for_path(&chunk.chunk.path);
        let snippet = snippet_first_chars(&chunk.chunk.text, 160);
        let json_ld = build_json_ld(chunk, language, &snippet);
        let n_chars = chunk.chunk.text.chars().count();

        let mut ctx = tera::Context::new();
        ctx.insert("site_title", site_title);
        ctx.insert("chunk", &chunk.chunk);
        ctx.insert("encoded_id", &encode_id(&chunk.chunk.id));
        ctx.insert("language", language);
        ctx.insert("snippet", &snippet);
        ctx.insert("json_ld", &json_ld);
        ctx.insert("siblings", &siblings);
        ctx.insert("n_chars", &n_chars);

        let html = tera
            .render("code_chunk.html", &ctx)
            .with_context(|| format!("rendering code chunk {}", chunk.chunk.id))?;
        let out = code_dir.join(format!("{}.html", encode_id(&chunk.chunk.id)));
        fs::write(&out, html).with_context(|| format!("writing {}", out.display()))?;
    }

    // Code index page (grouped by project → file).
    let mut by_project: BTreeMap<String, Vec<&IndexedChunk>> = BTreeMap::new();
    for chunk in &ci.chunks {
        by_project
            .entry(chunk.chunk.project.clone())
            .or_default()
            .push(chunk);
    }
    let projects: Vec<CodeProjectGroup> = by_project
        .into_iter()
        .map(|(name, chunks)| {
            let mut files: BTreeMap<String, Vec<CodeChunkSummary>> = BTreeMap::new();
            for c in &chunks {
                files
                    .entry(c.chunk.path.clone())
                    .or_default()
                    .push(CodeChunkSummary {
                        encoded_id: encode_id(&c.chunk.id),
                        start_line: c.chunk.start_line,
                        end_line: c.chunk.end_line,
                    });
            }
            for v in files.values_mut() {
                v.sort_by_key(|c| c.start_line);
            }
            let file_list: Vec<CodeFileGroup> = files
                .into_iter()
                .map(|(path, chunks)| CodeFileGroup { path, chunks })
                .collect();
            let n_chunks = chunks.len();
            CodeProjectGroup {
                name,
                files: file_list,
                n_chunks,
            }
        })
        .collect();

    let mut ctx = tera::Context::new();
    ctx.insert("site_title", site_title);
    ctx.insert("n_chunks", &ci.chunks.len());
    ctx.insert("model", &ci.model);
    ctx.insert("dim", &ci.dim);
    ctx.insert("projects", &projects);
    let html = tera.render("code_index.html", &ctx)?;
    fs::write(code_dir.join("index.html"), html)?;

    Ok(())
}

fn encode_id(id: &str) -> String {
    id.replace('/', "__").replace('#', "~~").replace('\\', "__")
}

fn snippet_first_chars(text: &str, max: usize) -> String {
    let cleaned: String = text.chars().take(max * 2).collect::<String>().replace('\n', " ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut s: String = cleaned.chars().take(max).collect();
        s.push('…');
        s
    }
}

fn language_for_path(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "md" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" => "html",
        "css" => "css",
        "go" => "go",
        "java" => "java",
        "cpp" | "cc" => "cpp",
        "c" | "h" | "hpp" => "c",
        "rb" => "ruby",
        "sh" | "bash" => "bash",
        "ps1" => "powershell",
        _ => "plaintext",
    }
}

fn build_json_ld(chunk: &IndexedChunk, lang: &str, snippet: &str) -> String {
    let encoded = encode_id(&chunk.chunk.id);
    let value = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "SoftwareSourceCode",
        "@id": chunk.chunk.id,
        "name": format!("{}:{}-{}", chunk.chunk.path, chunk.chunk.start_line, chunk.chunk.end_line),
        "description": snippet,
        "programmingLanguage": lang,
        "codeSampleType": "snippet",
        "isPartOf": {
            "@type": "SoftwareApplication",
            "name": chunk.chunk.project,
            "url": format!("../notes/project-{}.html", chunk.chunk.project),
        },
        "url": format!("./{}.html", encoded),
        "text": chunk.chunk.text,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn categorise(notes: &[Note]) -> Vec<CategoryGroup> {
    let mut buckets: BTreeMap<String, Vec<NoteSummary>> = BTreeMap::new();
    for n in notes {
        let t = n.note_type().to_string();
        buckets.entry(t).or_default().push(NoteSummary {
            slug: n.slug.clone(),
            title: n.title.clone(),
            description: n.description().to_string(),
            status: n.status().unwrap_or("").to_string(),
        });
    }
    for v in buckets.values_mut() {
        v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    }
    let mut ordered: Vec<CategoryGroup> = Vec::new();
    for cat in CATEGORY_ORDER {
        if let Some(v) = buckets.remove(*cat) {
            ordered.push(CategoryGroup {
                category: cat.to_string(),
                notes: v,
            });
        }
    }
    let mut leftovers: Vec<_> = buckets.into_iter().collect();
    leftovers.sort_by(|a, b| a.0.cmp(&b.0));
    for (category, notes) in leftovers {
        ordered.push(CategoryGroup { category, notes });
    }
    ordered
}

fn write_llms_txt(
    notes: &[Note],
    code_index: Option<&CodeIndex>,
    site_title: &str,
    output_dir: &Path,
) -> Result<()> {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", site_title));
    s.push_str("> Static knowledge base mirror of `~/.claude` memory plus indexed code chunks. Each entry below is a separate HTML page with full content. JSON-LD `SoftwareSourceCode` structured data on code pages; semantic HTML throughout.\n\n");

    s.push_str("## Notes\n\n");
    for n in notes {
        s.push_str(&format!(
            "- [{}](notes/{}.html): {}\n",
            n.title,
            n.slug,
            n.description()
        ));
    }

    if let Some(ci) = code_index {
        s.push_str(&format!(
            "\n## Code Chunks ({} total, model={}, dim={})\n\n",
            ci.chunks.len(),
            ci.model,
            ci.dim
        ));
        for c in &ci.chunks {
            let encoded = encode_id(&c.chunk.id);
            let preview = snippet_first_chars(&c.chunk.text, 120);
            s.push_str(&format!(
                "- [{}/{}:{}-{}](code/{}.html): {}\n",
                c.chunk.project,
                c.chunk.path,
                c.chunk.start_line,
                c.chunk.end_line,
                encoded,
                preview,
            ));
        }
    }

    fs::write(output_dir.join("llms.txt"), s).context("writing llms.txt")?;
    Ok(())
}
