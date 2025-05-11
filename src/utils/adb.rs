use crate::manifest::Component;
use anyhow::Result;
use crate::llm::analyzer::IntentParameter;

pub struct ADBCommand {
    component: Option<Component>,
    intent_params: Vec<IntentParameter>,
    extra_args: Vec<String>,
}

impl ADBCommand {
    pub fn new() -> Result<Self> {
        Ok(Self {
            component: None,
            intent_params: Vec::new(),
            extra_args: Vec::new(),
        })
    }

    pub fn set_component(&mut self, component: &Component) {
        self.component = Some(component.clone());
    }

    pub fn set_intent_params(&mut self, params: &[IntentParameter]) {
        self.intent_params = params.to_vec();
    }

    pub fn add_extra_arg(&mut self, arg: &str) {
        self.extra_args.push(arg.to_string());
    }

    pub fn build_command(&self) -> Result<String> {
        let component = self.component.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No component set"))?;

        // 컴포넌트 이름이 패키지명으로 시작하는지 확인
        let component_name = if component.name.starts_with(&component.package) {
            // 패키지명으로 시작하면 패키지명을 제외한 나머지 부분만 사용
            component.name[component.package.len()..].trim_start_matches('.')
        } else {
            // 패키지명으로 시작하지 않으면 전체 이름 사용
            &component.name
        };

        // 컴포넌트 이름이 '.'으로 시작하지 않는 경우 추가
        let component_name = if !component_name.starts_with('.') {
            format!(".{}", component_name)
        } else {
            component_name.to_string()
        };

        // 컴포넌트 이름이 패키지명을 포함하는지 한 번 더 확인
        let final_component_name = if component_name.contains(&component.package) {
            // 패키지명을 제외한 부분만 사용
            component_name.split(&component.package)
                .last()
                .unwrap_or(&component_name)
                .trim_start_matches('.')
                .to_string()
        } else {
            component_name
        };

        let mut command = format!(
            "adb shell am start -n {}/{}",
            component.package,
            final_component_name
        );

        // Add intent parameters
        for param in &self.intent_params {
            command.push(' ');
            command.push_str(&param.to_string());
        }

        // Add extra arguments
        for arg in &self.extra_args {
            command.push_str(&format!(" {}", arg));
        }

        Ok(command)
    }
}

#[allow(dead_code)]
pub fn generate_adb_commands(component: &Component) -> Vec<String> {
    let mut commands = Vec::new();
    let component_type = match component.component_type.as_str() {
        "activity" => "activity",
        "service" => "service",
        "receiver" => "broadcast",
        "provider" => "content",
        _ => "activity",
    };

    let component_name = if component.name.starts_with('.') {
        format!("{}{}", component.package, component.name)
    } else {
        component.name.clone()
    };

    // 기본 명령어 생성
    let base_command = match component_type {
        "activity" => format!("adb shell am start -n {}/{}", component.package, component_name),
        "service" => format!("adb shell am startservice -n {}/{}", component.package, component_name),
        "broadcast" => format!("adb shell am broadcast -n {}/{}", component.package, component_name),
        "content" => format!("adb shell content call --uri content://{}/{}", component.package, component_name),
        _ => return commands,
    };

    // action과 category가 없는 경우 기본 명령어만 추가
    if component.actions.is_empty() && component.categories.is_empty() && 
       component.data_schemes.is_empty() && component.data_mime_types.is_empty() {
        commands.push(base_command);
        return commands;
    }

    // action과 category의 모든 조합으로 명령어 생성
    for action in &component.actions {
        let mut command = base_command.clone();
        
        // action 추가
        if component_type == "broadcast" {
            command = format!("{} -a {}", command, action);
        } else {
            command = format!("{} -a {}", command, action);
        }

        // 데이터 URI 추가 (scheme, host, path)
        if !component.data_schemes.is_empty() {
            for scheme in &component.data_schemes {
                let data_uri = format!("{}", scheme);
                
                // host 추가
                if !component.data_hosts.is_empty() {
                    for host in &component.data_hosts {
                        let host_uri = format!("{}://{}", data_uri, host);
                        
                        // path 추가
                        if !component.data_paths.is_empty() {
                            for path in &component.data_paths {
                                let full_uri = format!("{}{}", host_uri, path);
                                let data_command = format!("{} -d \"{}\"", command, full_uri);
                                
                                // category 추가
                                if component.categories.is_empty() {
                                    commands.push(data_command.clone());
                                } else {
                                    for category in &component.categories {
                                        let category_command = if component_type == "broadcast" {
                                            format!("{} -c {}", data_command, category)
                                        } else {
                                            format!("{} -c {}", data_command, category)
                                        };
                                        commands.push(category_command);
                                    }
                                }
                            }
                        } else {
                            // path가 없는 경우
                            let data_command = format!("{} -d \"{}\"", command, host_uri);
                            
                            // category 추가
                            if component.categories.is_empty() {
                                commands.push(data_command.clone());
                            } else {
                                for category in &component.categories {
                                    let category_command = if component_type == "broadcast" {
                                        format!("{} -c {}", data_command, category)
                                    } else {
                                        format!("{} -c {}", data_command, category)
                                    };
                                    commands.push(category_command);
                                }
                            }
                        }
                    }
                } else {
                    // host가 없는 경우
                    let data_command = format!("{} -d \"{}://\"", command, data_uri);
                    
                    // category 추가
                    if component.categories.is_empty() {
                        commands.push(data_command.clone());
                    } else {
                        for category in &component.categories {
                            let category_command = if component_type == "broadcast" {
                                format!("{} -c {}", data_command, category)
                            } else {
                                format!("{} -c {}", data_command, category)
                            };
                            commands.push(category_command);
                        }
                    }
                }
            }
        } else if !component.data_mime_types.is_empty() {
            // MIME 타입 추가
            for mime_type in &component.data_mime_types {
                let mime_command = format!("{} -t \"{}\"", command, mime_type);
                
                // category 추가
                if component.categories.is_empty() {
                    commands.push(mime_command.clone());
                } else {
                    for category in &component.categories {
                        let category_command = if component_type == "broadcast" {
                            format!("{} -c {}", mime_command, category)
                        } else {
                            format!("{} -c {}", mime_command, category)
                        };
                        commands.push(category_command);
                    }
                }
            }
        } else {
            // 데이터 URI나 MIME 타입이 없는 경우
            // category가 없는 경우 현재 action만으로 명령어 추가
            if component.categories.is_empty() {
                commands.push(command);
                continue;
            }

            // 각 category에 대해 명령어 생성
            for category in &component.categories {
                let category_command = if component_type == "broadcast" {
                    format!("{} -c {}", command, category)
                } else {
                    format!("{} -c {}", command, category)
                };
                commands.push(category_command);
            }
        }
    }

    // action이 없지만 data_schemes가 있는 경우에 대한 처리 추가
    if component.actions.is_empty() && !component.data_schemes.is_empty() {
        let command = base_command.clone();
        
        for scheme in &component.data_schemes {
            let data_uri = format!("{}", scheme);
            
            // host 추가
            if !component.data_hosts.is_empty() {
                for host in &component.data_hosts {
                    let host_uri = format!("{}://{}", data_uri, host);
                    
                    // path 추가
                    if !component.data_paths.is_empty() {
                        for path in &component.data_paths {
                            let full_uri = format!("{}{}", host_uri, path);
                            let data_command = format!("{} -d \"{}\"", command, full_uri);
                            
                            // category 추가
                            if component.categories.is_empty() {
                                commands.push(data_command.clone());
                            } else {
                                for category in &component.categories {
                                    let category_command = format!("{} -c {}", data_command, category);
                                    commands.push(category_command);
                                }
                            }
                        }
                    } else {
                        // path가 없는 경우
                        let data_command = format!("{} -d \"{}\"", command, host_uri);
                        
                        // category 추가
                        if component.categories.is_empty() {
                            commands.push(data_command.clone());
                        } else {
                            for category in &component.categories {
                                let category_command = format!("{} -c {}", data_command, category);
                                commands.push(category_command);
                            }
                        }
                    }
                }
            } else {
                // host가 없는 경우
                let data_command = format!("{} -d \"{}://\"", command, data_uri);
                
                // category 추가
                if component.categories.is_empty() {
                    commands.push(data_command.clone());
                } else {
                    for category in &component.categories {
                        let category_command = format!("{} -c {}", data_command, category);
                        commands.push(category_command);
                    }
                }
            }
        }
    }

    commands
} 