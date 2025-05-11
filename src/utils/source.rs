use std::path::{PathBuf, Path};
use std::collections::HashMap;
use anyhow::{Result, Context};
use crate::manifest::Component;
use tree_sitter::{Parser, Query, QueryCursor};
use tree_sitter_java::language;

#[derive(Debug)]
pub struct IntentParameter {
    pub name: String,
    pub value: String,
    pub type_: String,
}

pub struct SourceFileCache {
    files: HashMap<String, Vec<PathBuf>>,
    manifest_dir: PathBuf,
}

impl SourceFileCache {
    pub fn new(manifest_path: &PathBuf) -> Self {
        Self {
            files: HashMap::new(),
            manifest_dir: manifest_path.parent().unwrap().to_path_buf(),
        }
    }

    pub fn scan_directory(&mut self, dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let entries = std::fs::read_dir(dir)?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                self.scan_directory(&path)?;
            } else if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if ext_str == "java" || ext_str == "kt" {
                    if let Some(file_name) = path.file_stem() {
                        let name = file_name.to_string_lossy().to_string();
                        self.files.entry(name).or_default().push(path);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn find_component_file(&self, component: &Component) -> Option<PathBuf> {
        let component_name = component.name.split('.').last().unwrap_or(&component.name);
        
        // 1. Exact name matching
        if let Some(files) = self.files.get(component_name) {
            if files.len() == 1 {
                return Some(files[0].clone());
            }
        }

        // 2. Partial name matching
        let mut matches = Vec::new();
        for (name, files) in &self.files {
            if name.contains(component_name) || component_name.contains(name) {
                matches.extend(files.clone());
            }
        }

        // Filter files that are under the manifest directory
        let manifest_matches: Vec<_> = matches.into_iter()
            .filter(|path| path.starts_with(&self.manifest_dir))
            .collect();

        if manifest_matches.len() == 1 {
            Some(manifest_matches[0].clone())
        } else {
            None
        }
    }
}

pub fn find_source_dir(manifest_path: &PathBuf) -> Option<PathBuf> {
    let mut cache = SourceFileCache::new(manifest_path);
    let manifest_dir = cache.manifest_dir.clone();
    
    // Scan manifest directory and its subdirectories
    if let Err(e) = cache.scan_directory(&manifest_dir) {
        eprintln!("Error scanning source files: {}", e);
        return None;
    }

    // Check if any files were found
    if cache.files.is_empty() {
        return None;
    }

    // Return the parent directory of the first file
    cache.files.values().next()
        .and_then(|files| files.first())
        .and_then(|path| path.parent())
        .map(|p| p.to_path_buf())
}

pub fn find_source_file(component: &Component, _base_dir: &str) -> Result<PathBuf> {
    let manifest_path = PathBuf::from(&component.manifest_path);
    let mut cache = SourceFileCache::new(&manifest_path);
    let manifest_dir = cache.manifest_dir.clone();
    
    // Scan the manifest directory and its subdirectories
    cache.scan_directory(&manifest_dir)
        .map_err(|e| anyhow::anyhow!("Failed to scan directory for source files: {}", e))?;

    // Try to find the component's source file
    cache.find_component_file(component)
        .ok_or_else(|| anyhow::anyhow!("Could not find source file for component: {}", component.name))
}

pub fn parse_intent_parameters(source_file: &PathBuf) -> Result<Vec<IntentParameter>> {
    let source_code = std::fs::read_to_string(source_file)
        .map_err(|e| anyhow::anyhow!("Failed to read source file: {}", e))?;

    let mut parser = Parser::new();
    parser.set_language(language())
        .expect("Error loading Java parser");

    let tree = parser.parse(&source_code, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse source code"))?;

    // Query to find getIntent() calls and their parameters
    let query = Query::new(
        language(),
        r#"
        (method_invocation
            name: (identifier) @method_name
            arguments: (argument_list) @args
            (#eq? @method_name "getIntent")
        )
        "#
    ).expect("Failed to create query");

    let mut cursor = QueryCursor::new();
    let mut parameters = Vec::new();
    let matches = cursor.matches(&query, tree.root_node(), source_code.as_bytes());
    
    // Iterate over matches using Iterator trait (tree-sitter 0.20.9)
    for m in matches {
        for capture in m.captures {
            if capture.index == 1 { // args capture
                let args_node = capture.node;
                let args_text = args_node.utf8_text(source_code.as_bytes())
                    .expect("Failed to get args text");

                // Parse each argument
                for child in args_node.children(&mut args_node.walk()) {
                    if child.kind() == "assignment_expression" {
                        let name_node = child.child_by_field_name("left")
                            .expect("Failed to get name node");
                        let value_node = child.child_by_field_name("right")
                            .expect("Failed to get value node");

                        let name = name_node.utf8_text(source_code.as_bytes())
                            .expect("Failed to get name text")
                            .trim()
                            .to_string();

                        let value = value_node.utf8_text(source_code.as_bytes())
                            .expect("Failed to get value text")
                            .trim()
                            .to_string();

                        // Determine parameter type based on value
                        let type_ = if value.starts_with("\"") {
                            "string".to_string()
                        } else if value.parse::<i64>().is_ok() {
                            "int".to_string()
                        } else if value.parse::<f64>().is_ok() {
                            "float".to_string()
                        } else if value == "true" || value == "false" {
                            "boolean".to_string()
                        } else {
                            "unknown".to_string()
                        };

                        println!("name: {}, value: {}, type_: {}", name, value, type_);
                        parameters.push(IntentParameter {
                            name,
                            value,
                            type_,
                        });
                    }
                }
            }
        }
    }

    Ok(parameters)
}

pub fn intent_parameters_to_adb_args(parameters: &[IntentParameter]) -> Vec<String> {
    parameters.iter()
        .map(|param| {
            match param.type_.as_str() {
                "string" => format!("--es {} {}", param.name, param.value.trim_matches('"')),
                "int" => format!("--ei {} {}", param.name, param.value),
                "float" => format!("--ef {} {}", param.name, param.value),
                "boolean" => format!("--ez {} {}", param.name, param.value),
                _ => format!("--es {} {}", param.name, param.value),
            }
        })
        .collect()
} 