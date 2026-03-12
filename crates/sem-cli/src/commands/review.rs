use std::path::Path;
use std::process;

use colored::Colorize;
use sem_core::git::bridge::GitBridge;
use sem_core::git::types::DiffScope;
use sem_core::model::change::ChangeType;
use sem_core::parser::differ::compute_semantic_diff;
use sem_core::parser::plugins::create_default_registry;

use crate::formatters::terminal::format_terminal;
use crate::commands::diff::OutputFormat;

pub struct ReviewOptions {
    pub cwd: String,
    pub format: OutputFormat,
    pub base: String,
    pub file_exts: Vec<String>,
}

pub fn review_command(branch: &str, opts: ReviewOptions) {
    let git = match GitBridge::open(Path::new(&opts.cwd)) {
        Ok(g) => g,
        Err(_) => {
            eprintln!("{}", "Error: Not inside a Git repository.".red());
            process::exit(1);
        }
    };

    let scope = DiffScope::Range {
        from: opts.base.clone(),
        to: branch.to_string(),
    };

    let file_changes = match git.get_changed_files(&scope) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("{}", format!("Error: {e}").red());
            process::exit(1);
        }
    };

    // Filter by file extensions if specified
    let file_changes: Vec<_> = if opts.file_exts.is_empty() {
        file_changes
    } else {
        let exts: Vec<String> = opts.file_exts.iter().map(|e| {
            if e.starts_with('.') { e.clone() } else { format!(".{}", e) }
        }).collect();
        file_changes.into_iter().filter(|fc| {
            exts.iter().any(|ext| fc.file_path.ends_with(ext.as_str()))
        }).collect()
    };

    if file_changes.is_empty() {
        println!("{}", "No changes between branches.".dimmed());
        return;
    }

    let registry = create_default_registry();
    let result = compute_semantic_diff(&file_changes, &registry, None, None);

    if opts.format == OutputFormat::Json {
        let changes_json: Vec<serde_json::Value> = result.changes.iter().map(|c| {
            serde_json::json!({
                "entityId": c.entity_id,
                "changeType": c.change_type,
                "entityType": c.entity_type,
                "entityName": c.entity_name,
                "filePath": c.file_path,
                "oldFilePath": c.old_file_path,
                "beforeContent": c.before_content,
                "afterContent": c.after_content,
            })
        }).collect();

        let output = serde_json::json!({
            "base": opts.base,
            "target": branch,
            "summary": {
                "fileCount": result.file_count,
                "added": result.added_count,
                "modified": result.modified_count,
                "deleted": result.deleted_count,
                "moved": result.moved_count,
                "renamed": result.renamed_count,
                "total": result.changes.len(),
            },
            "changes": changes_json,
        });

        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        return;
    }

    // Terminal output
    println!("{}", format!("\n  Review: {} ← {}\n", opts.base, branch).dimmed());

    println!("{}", format_terminal(&result));

    // Risk assessment
    let mut risks: Vec<String> = Vec::new();

    let deleted_functions: Vec<_> = result.changes.iter().filter(|c| {
        c.change_type == ChangeType::Deleted
            && (c.entity_type == "function" || c.entity_type == "method")
    }).collect();

    if !deleted_functions.is_empty() {
        let names: Vec<&str> = deleted_functions.iter().map(|f| f.entity_name.as_str()).collect();
        risks.push(format!(
            "  {}  {} function{} deleted: {}",
            "⚠".red(),
            deleted_functions.len(),
            if deleted_functions.len() > 1 { "s" } else { "" },
            names.join(", "),
        ));
    }

    let modified_configs = result.changes.iter().filter(|c| {
        c.change_type == ChangeType::Modified
            && (c.entity_type == "property" || c.entity_type == "section")
    }).count();

    if modified_configs > 5 {
        risks.push(format!(
            "  {}  {} config properties changed — verify production settings",
            "⚠".yellow(),
            modified_configs,
        ));
    }

    if result.changes.len() > 50 {
        risks.push(format!(
            "  {}  Large changeset ({} entities) — consider splitting",
            "⚠".yellow(),
            result.changes.len(),
        ));
    }

    if !risks.is_empty() {
        println!("{}", "\nRisk signals:".bold());
        for risk in &risks {
            println!("{risk}");
        }
        println!();
    }
}
