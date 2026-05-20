//! Build the knowledge graph: notes + (optionally) code chunks.

use crate::parser::Note;
use crate::CodeIndex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub note_type: String,
    pub status: String,
    pub source: String,
    pub is_project: bool,
    pub dangling: bool,
    /// True for code-chunk nodes (separate styling in the UI).
    pub is_code: bool,
    /// Brief snippet preview (first ~120 chars) — surfaced in hover card.
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

pub fn build_graph(notes: &[Note], code_index: Option<&CodeIndex>) -> Graph {
    let mut nodes: Vec<GraphNode> = notes
        .iter()
        .map(|n| GraphNode {
            id: n.slug.clone(),
            label: n.title.clone(),
            note_type: n.note_type().to_string(),
            status: n.status().unwrap_or("").to_string(),
            source: n.source.clone(),
            is_project: n.note_type() == "project",
            dangling: false,
            is_code: false,
            snippet: n.description().to_string(),
        })
        .collect();

    let mut known: HashSet<String> = notes.iter().map(|n| n.slug.clone()).collect();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut dangling_targets: HashSet<String> = HashSet::new();

    for n in notes {
        for target in &n.outgoing_links {
            edges.push(GraphEdge {
                source: n.slug.clone(),
                target: target.clone(),
            });
            if !known.contains(target) {
                dangling_targets.insert(target.clone());
            }
        }
    }

    for slug in dangling_targets {
        nodes.push(GraphNode {
            id: slug.clone(),
            label: slug.replace('-', " "),
            note_type: "dangling".into(),
            status: "".into(),
            source: "".into(),
            is_project: false,
            dangling: true,
            is_code: false,
            snippet: "".into(),
        });
        known.insert(slug);
    }

    // Code chunks as additional nodes — each linked to its project memory
    // node if one exists (slug convention: `project-<source-name>`).
    if let Some(ci) = code_index {
        for ic in &ci.chunks {
            let label = format!(
                "{}:{}-{}",
                ic.chunk.path, ic.chunk.start_line, ic.chunk.end_line
            );
            let snippet: String = ic
                .chunk
                .text
                .chars()
                .take(120)
                .collect::<String>()
                .replace('\n', " ");
            nodes.push(GraphNode {
                id: ic.chunk.id.clone(),
                label,
                note_type: "code".into(),
                status: "".into(),
                source: ic.chunk.project.clone(),
                is_project: false,
                dangling: false,
                is_code: true,
                snippet,
            });
        }
        for ic in &ci.chunks {
            // Connect each chunk to its project memory node if present.
            let proj_slug = format!("project-{}", ic.chunk.project);
            if known.contains(&proj_slug) {
                edges.push(GraphEdge {
                    source: ic.chunk.id.clone(),
                    target: proj_slug,
                });
            }
        }
    }

    Graph { nodes, edges }
}

pub fn build_backlinks(notes: &[Note]) -> HashMap<String, Vec<&Note>> {
    let mut backlinks: HashMap<String, Vec<&Note>> = HashMap::new();
    for n in notes {
        for target in &n.outgoing_links {
            backlinks.entry(target.clone()).or_default().push(n);
        }
    }
    for v in backlinks.values_mut() {
        v.sort_by(|a, b| a.slug.cmp(&b.slug));
    }
    backlinks
}

/// Project-only subgraph: just project nodes + edges among them.
pub fn project_subgraph(full: &Graph) -> Graph {
    let project_ids: HashSet<&str> = full
        .nodes
        .iter()
        .filter(|n| n.is_project)
        .map(|n| n.id.as_str())
        .collect();

    let nodes: Vec<GraphNode> = full
        .nodes
        .iter()
        .filter(|n| n.is_project)
        .map(|n| GraphNode {
            id: n.id.clone(),
            label: n.label.clone(),
            note_type: n.note_type.clone(),
            status: n.status.clone(),
            source: n.source.clone(),
            is_project: true,
            dangling: n.dangling,
            is_code: false,
            snippet: n.snippet.clone(),
        })
        .collect();

    let edges: Vec<GraphEdge> = full
        .edges
        .iter()
        .filter(|e| {
            project_ids.contains(e.source.as_str()) && project_ids.contains(e.target.as_str())
        })
        .map(|e| GraphEdge {
            source: e.source.clone(),
            target: e.target.clone(),
        })
        .collect();

    Graph { nodes, edges }
}
