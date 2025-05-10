use std::collections::HashSet;
use std::path::Path;
use tracing::info;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,           // 전체 이름 (package.class_name)
    pub class_name: String,     // 클래스 이름만
    pub package: String,        // 패키지 이름
    pub component_type: String,
    pub exported: bool,
    pub actions: HashSet<String>,
    pub categories: HashSet<String>,
    pub data_schemes: HashSet<String>,
    pub data_hosts: HashSet<String>,
    pub data_paths: HashSet<String>,
    pub data_mimeTypes: HashSet<String>,
    pub permissions: Vec<String>,
    pub intent_filter_permissions: Vec<String>,
    pub manifest_dir: PathBuf,
    pub shared_user_id: Option<String>,
    pub manifest_path: PathBuf,  // AndroidManifest.xml 파일 경로
    pub manifest_line: usize,    // 컴포넌트 선언의 줄 번호
    pub xml_element: Option<String>,
}

impl Component {
    pub fn new(
        name: String,
        package: String,
        component_type: String,
        exported: bool,
        manifest_dir: PathBuf,
        manifest_path: PathBuf,
        manifest_line: usize,
        class_name: String,
        actions: Vec<String>,
        categories: Vec<String>,
        data_schemes: Vec<String>,
        data_hosts: Vec<String>,
        data_paths: Vec<String>,
        data_mimeTypes: Vec<String>,
        permissions: Vec<String>,
        intent_filter_permissions: Vec<String>,
        shared_user_id: Option<String>,
        xml_element: Option<String>,
    ) -> Self {
        Self {
            name,
            class_name,
            package,
            component_type,
            exported,
            actions: HashSet::from_iter(actions),
            categories: HashSet::from_iter(categories),
            data_schemes: HashSet::from_iter(data_schemes),
            data_hosts: HashSet::from_iter(data_hosts),
            data_paths: HashSet::from_iter(data_paths),
            data_mimeTypes: HashSet::from_iter(data_mimeTypes),
            permissions,
            intent_filter_permissions,
            manifest_dir,
            shared_user_id,
            manifest_path,
            manifest_line,
            xml_element,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        // Extract package and component name from path
        let path_str = path.to_string_lossy();
        info!("Analyzing path: {}", path_str);
        
        let parts: Vec<&str> = path_str.split('/').collect();
        
        if parts.len() < 2 {
            info!("Path too short: {}", path_str);
            return None;
        }

        // Try to determine component type from path
        let component_type = if path_str.contains("/activity/") {
            "activity"
        } else if path_str.contains("/service/") {
            "service"
        } else if path_str.contains("/receiver/") {
            "receiver"
        } else if path_str.contains("/provider/") {
            "provider"
        } else {
            info!("No component type found in path: {}", path_str);
            return None; // Skip if we can't determine the type
        };

        let package = parts[parts.len() - 2].to_string();
        let name = parts[parts.len() - 1].to_string();

        // Skip if name doesn't end with .java or .kt
        if !name.ends_with(".java") && !name.ends_with(".kt") {
            info!("Not a Java/Kotlin file: {}", name);
            return None;
        }

        // Remove file extension from name
        let class_name = name.split('.').next().unwrap_or(&name).to_string();
        let full_name = format!("{}.{}", package, class_name);
        info!("Found component: {} of type {} in package {}", class_name, component_type, package);

        Some(Self {
            name: full_name,
            class_name,
            package,
            component_type: component_type.to_string(),
            exported: false,
            actions: HashSet::new(),
            categories: HashSet::new(),
            data_schemes: HashSet::new(),
            data_hosts: HashSet::new(),
            data_paths: HashSet::new(),
            data_mimeTypes: HashSet::new(),
            permissions: Vec::new(),
            intent_filter_permissions: Vec::new(),
            manifest_dir: PathBuf::new(),
            shared_user_id: None,
            manifest_path: PathBuf::new(),
            manifest_line: 0,
            xml_element: None,
        })
    }

    pub fn set_shared_user_id(&mut self, shared_user_id: String) {
        self.shared_user_id = Some(shared_user_id);
    }
} 