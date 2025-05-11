use std::path::PathBuf;
use std::collections::{HashMap, HashSet};
use anyhow::Result;
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

#[allow(dead_code)]
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

    // Refined query to find only `get.*Extra` methods and `getData`
    let query = Query::new(
        language(),
        r#"
        ;; Capture any method invocation that retrieves extras or data from an Intent
        (method_invocation
            object: (identifier) @intent_var
            name: (identifier) @extra_method
            arguments: (argument_list) @args
            (#match? @extra_method "^get.*Extra$")
        )

        ;; Capture method invocations for getData
        (method_invocation
            object: (identifier) @intent_var
            name: (identifier) @data_method
            (#eq? @data_method "getData")
        )
        "#
    ).expect("Failed to create query");

    let mut cursor = QueryCursor::new();
    let mut parameters = Vec::new();
    let matches = cursor.matches(&query, tree.root_node(), source_code.as_bytes());

    // Iterate over matches using Iterator trait (tree-sitter 0.20.9)
    for m in matches {
        let mut args_node = None;
        let mut method_name = None;

        // Capture method names and arguments
        for capture in m.captures {
            match capture.node.kind() {
                "argument_list" => args_node = Some(capture.node),
                "identifier" => {
                    method_name = Some(capture.node.utf8_text(source_code.as_bytes()).unwrap_or("").to_string());
                },
                _ => {}
            }
        }

        if let Some(method_name) = method_name {
            // Special case for getData()
            if method_name == "getData" {
                // Handle getData() case
                let param_id = format!("data:uri:{}", method_name);
                parameters.push(IntentParameter {
                    name: "data".to_string(),
                    value: "uri".to_string(),
                    type_: "uri".to_string(),
                });
                continue;
            }

            if let Some(args_node) = args_node {
                // Extract parameter name and default value if any
                let args = args_node.children(&mut args_node.walk())
                    .filter(|n| n.kind() != "(" && n.kind() != ")" && n.kind() != ",")
                    .collect::<Vec<_>>();
                    
                if args.is_empty() {
                    continue;
                }
                
                // Get parameter key name
                let key_node = &args[0];
                let key = key_node.utf8_text(source_code.as_bytes())
                    .unwrap_or("unknown")
                    .trim_matches('"')
                    .to_string();
                    
                // Determine parameter type based on method name
                let type_ = if method_name.contains("String") {
                    "string".to_string()
                } else if method_name.contains("Int") {
                    "int".to_string()
                } else if method_name.contains("Float") || method_name.contains("Double") {
                    "float".to_string()
                } else if method_name.contains("Boolean") {
                    "boolean".to_string()
                } else {
                    "unknown".to_string()
                };
                
                // Get default value if provided, otherwise use type as default
                let value = if args.len() > 1 {
                    args[1].utf8_text(source_code.as_bytes())
                        .unwrap_or(&type_)
                        .to_string()
                } else {
                    type_.clone()
                };
                
                // Create unique identifier for parameter to avoid duplicates
                // Include method name in the param_id to better handle duplicates
                let param_id = format!("{}:{}:{}", key, type_, method_name);
                parameters.push(IntentParameter {
                    name: key,
                    value,
                    type_,
                });
            }
        }
    }

    Ok(parameters)
}

pub fn intent_parameters_to_adb_args(parameters: &[IntentParameter]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen_params = std::collections::HashSet::new();
    
    for param in parameters {
        let param_key = format!("{}:{}", param.name, param.type_);
        if seen_params.contains(&param_key) {
            continue;
        }
        
        seen_params.insert(param_key);
        
        let arg = match param.type_.as_str() {
            "string" => format!("--es {} {}", param.name, param.value.trim_matches('"')),
            "int" => format!("--ei {} {}", param.name, param.value),
            "float" => format!("--ef {} {}", param.name, param.value),
            "boolean" => format!("--ez {} {}", param.name, param.value),
            _ => format!("--es {} {}", param.name, param.value),
        };
        
        result.push(arg);
    }
    
    result
}
