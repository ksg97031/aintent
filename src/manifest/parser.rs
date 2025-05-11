use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::collections::HashSet;
use xml::reader::{EventReader, XmlEvent};
use crate::manifest::component::Component;

pub fn find_manifest_files(dir: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path().to_string_lossy();
            !path.contains("test") && e.file_name() == "AndroidManifest.xml"
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn parse_manifest(file_path: &PathBuf, package_filter: Option<&str>) -> Result<Vec<Component>, Box<dyn std::error::Error>> {
    let file = File::open(file_path)?;
    let file = BufReader::new(file);
    let parser = EventReader::new(file);
    
    let mut components = Vec::new();
    let mut current_package = String::new();
    let mut current_shared_user_id = None;
    let mut current_component = Option::<Component>::None;
    let mut current_actions = HashSet::new();
    let mut current_categories = HashSet::new();
    let mut current_data_schemes = HashSet::new();
    let mut current_data_hosts = HashSet::new();
    let mut current_data_paths = HashSet::new();
    let mut current_mime_types = HashSet::new();
    let _current_permissions: Vec<String> = Vec::new();
    let mut current_intent_filter_permissions = Vec::new();
    let mut in_intent_filter = false;
    let mut _depth = 0;
    let mut current_line = 0;
    let mut current_xml = String::new();

    // 매니페스트 디렉토리 경로 가져오기
    let manifest_dir = file_path.parent()
        .ok_or_else(|| "Failed to get manifest directory")?
        .to_path_buf();

    for event in parser {
        match event {
            Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                _depth += 1;
                current_line += 1;
                match name.local_name.as_str() {
                    "manifest" => {
                        for attr in attributes {
                            match attr.name.local_name.as_str() {
                                "package" => current_package = attr.value,
                                "sharedUserId" => current_shared_user_id = Some(attr.value),
                                _ => {}
                            }
                        }
                    }
                    "activity" | "service" | "receiver" | "provider" => {
                        let component_type = name.local_name.clone();
                        let mut component_name = String::new();
                        let mut exported = false;
                        current_xml = format!("<{}", name.local_name);

                        for attr in &attributes {
                            match attr.name.local_name.as_str() {
                                "name" => component_name = attr.value.clone(),
                                "exported" => exported = attr.value == "true",
                                _ => {}
                            }
                            current_xml.push_str(&format!(" {}={}", attr.name.local_name, attr.value));
                        }
                        current_xml.push('>');

                        if !component_name.is_empty() {
                            let full_name = if component_name.starts_with('.') {
                                format!("{}{}", current_package, component_name)
                            } else {
                                component_name
                            };

                            let component = Component {
                                name: full_name.clone(),
                                package: current_package.clone(),
                                component_type,
                                exported,
                                manifest_path: file_path.clone(),
                                manifest_line: current_line,
                                manifest_dir: manifest_dir.clone(),
                                class_name: full_name,
                                actions: HashSet::new(),
                                categories: HashSet::new(),
                                data_schemes: HashSet::new(),
                                data_hosts: HashSet::new(),
                                data_paths: HashSet::new(),
                                data_mime_types: HashSet::new(),
                                permissions: Vec::new(),
                                intent_filter_permissions: Vec::new(),
                                shared_user_id: current_shared_user_id.clone(),
                                xml_element: Some(current_xml.clone()),
                            };
                            current_component = Some(component);
                        }
                    }
                    "intent-filter" => {
                        in_intent_filter = true;
                        current_actions.clear();
                        current_categories.clear();
                        current_data_schemes.clear();
                        current_data_hosts.clear();
                        current_data_paths.clear();
                        current_mime_types.clear();
                        current_intent_filter_permissions.clear();
                    }
                    "action" => {
                        if in_intent_filter {
                            for attr in attributes {
                                if attr.name.local_name == "name" {
                                    current_actions.insert(attr.value);
                                }
                            }
                        }
                    }
                    "category" => {
                        if in_intent_filter {
                            for attr in attributes {
                                if attr.name.local_name == "name" {
                                    current_categories.insert(attr.value);
                                }
                            }
                        }
                    }
                    "data" => {
                        if in_intent_filter {
                            for attr in attributes {
                                match attr.name.local_name.as_str() {
                                    "scheme" => current_data_schemes.insert(attr.value),
                                    "host" => current_data_hosts.insert(attr.value),
                                    "path" => current_data_paths.insert(attr.value),
                                    "mimeType" => current_mime_types.insert(attr.value),
                                    _ => false,
                                };
                            }
                        }
                    }
                    "permission" => {
                        if in_intent_filter {
                            for attr in attributes {
                                if attr.name.local_name == "name" {
                                    current_intent_filter_permissions.push(attr.value);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::EndElement { name, .. }) => {
                _depth -= 1;
                match name.local_name.as_str() {
                    "activity" | "service" | "receiver" | "provider" => {
                        if let Some(mut component) = current_component.take() {
                            component.actions = current_actions.clone();
                            component.categories = current_categories.clone();
                            component.data_schemes = current_data_schemes.clone();
                            component.data_hosts = current_data_hosts.clone();
                            component.data_paths = current_data_paths.clone();
                            component.data_mime_types = current_mime_types.clone();
                            component.intent_filter_permissions = current_intent_filter_permissions.iter().cloned().collect();

                            if let Some(package) = &package_filter {
                                if component.package == *package {
                                    components.push(component);
                                }
                            } else {
                                components.push(component);
                            }
                        }
                    }
                    "intent-filter" => {
                        in_intent_filter = false;
                    }
                    _ => {}
                }
            }
            Err(e) => return Err(Box::new(e)),
            _ => {}
        }
    }

    Ok(components)
} 