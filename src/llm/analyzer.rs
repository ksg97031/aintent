use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::fmt;
use reqwest::Client;
use serde_json::{json, Value};
use anyhow::{Result, Context};
use tracing::{info, error, warn};
use crate::manifest::Component;
use super::config::LLMConfig;
use walkdir;

#[derive(Debug, Clone)]
pub struct IntentParameter {
    pub name: String,
    pub param_type: String,
    pub value: String,
    pub flag: String,  // -a, -c, -e 등의 플래그
}

impl fmt::Display for IntentParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.flag, self.value)
    }
}

pub struct IntentAnalysis {
    pub intent_params: Vec<IntentParameter>,
    pub confidence: f64,
    pub source_context: String,
}

pub async fn analyze_intent(
    _component: &Component,
    source_file: &str,
    config: &LLMConfig,
) -> Result<IntentAnalysis> {
    // 소스 파일 읽기
    let lines = read_source_file(source_file)?;
    
    // Intent 관련 코드 추출
    let context = extract_intent_context(&lines)?;
    
    // LLM API 호출
    let analysis = call_llm_api(&context, config).await?;
    
    // 결과 파싱 및 반환
    let params = parse_llm_response(&analysis)?;
    
    Ok(IntentAnalysis {
        intent_params: params,
        confidence: analysis["confidence"].as_f64().unwrap_or(0.0),
        source_context: context,
    })
}

fn read_source_file(source_file: &str) -> Result<Vec<String>> {
    let file = File::open(source_file)
        .context("Failed to open source file")?;
    let reader = BufReader::new(file);
    let lines = reader.lines()
        .collect::<io::Result<Vec<String>>>()
        .context("Failed to read source file")?;
    
    if lines.is_empty() {
        return Err(anyhow::anyhow!("Source file is empty"));
    }
    
    Ok(lines)
}

fn extract_intent_context(lines: &[String]) -> Result<String> {
    let mut has_intent = false;
    let mut context_lines = Vec::new();
    let mut in_intent_block = false;
    let mut block_depth = 0;
    let mut last_intent_line = 0;
    
    // Intent 관련 메서드 패턴
    let intent_patterns = [
        // Action 관련
        "getAction", "hasAction", "setAction",
        // Category 관련
        "getCategories", "hasCategory", "addCategory",
        // Data 관련
        "getData", "setData", "getScheme", "getHost", "getPath", "getQuery",
        // Type 관련
        "getType", "setType", "resolveType",
        // Extra 관련
        "getExtras", "getStringExtra", "getIntExtra", "getBooleanExtra",
        "getLongExtra", "getFloatExtra", "getDoubleExtra", "getParcelableExtra",
        // Flag 관련
        "getFlags", "addFlags", "setFlags",
        // Component 관련
        "getComponent", "setComponent", "resolveActivity",
        // URI 관련
        "toUri", "parseUri", "normalize",
        // Bundle 관련
        "getBundle", "putExtra", "putExtras"
    ];
    
    for (i, line) in lines.iter().enumerate() {
        let is_intent_line = intent_patterns.iter().any(|pattern| line.contains(pattern)) ||
            (line.contains("intent") && (
                line.contains(".get") || 
                line.contains(".set") || 
                line.contains(".has") || 
                line.contains(".add") ||
                line.contains(".put")
            )) ||
            (line.contains("getIntent()") && (
                line.contains(".get") || 
                line.contains(".set") || 
                line.contains(".has") || 
                line.contains(".add") ||
                line.contains(".put")
            ));

        if is_intent_line {
            has_intent = true;
            in_intent_block = true;
            block_depth = 0;
            last_intent_line = i;
            
            // 이전 컨텍스트 라인 추가 (최대 5줄)
            let start = i.saturating_sub(5);
            for j in start..i {
                if !context_lines.contains(&lines[j]) {
                    context_lines.push(lines[j].clone());
                }
            }
        }
        
        if in_intent_block {
            if !context_lines.contains(line) {
                context_lines.push(line.clone());
            }
            
            if line.contains('{') {
                block_depth += 1;
            }
            if line.contains('}') {
                block_depth -= 1;
                if block_depth == 0 {
                    in_intent_block = false;
                    // 이후 컨텍스트 라인 추가 (최대 5줄)
                    let end = (i + 6).min(lines.len());
                    for j in (i + 1)..end {
                        if !context_lines.contains(&lines[j]) {
                            context_lines.push(lines[j].clone());
                        }
                    }
                }
            }
        }
    }

    if !has_intent {
        info!("No intent-related code found in the source file");
        return Ok(String::new());
    }

    // 마지막 Intent 라인 이후의 컨텍스트 추가
    let end = (last_intent_line + 6).min(lines.len());
    for i in (last_intent_line + 1)..end {
        if !context_lines.contains(&lines[i]) {
            context_lines.push(lines[i].clone());
        }
    }

    Ok(context_lines.join("\n"))
}

async fn call_llm_api(context: &str, config: &LLMConfig) -> Result<Value> {
    let client = Client::new();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    
    if let Some(key) = &config.api_key {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", key).parse().unwrap(),
        );
    }

    let prompt = format!(
        "Analyze the following Android Intent code and extract all possible parameters for ADB command. Focus on:
1. Intent actions (getAction(), hasAction())
2. Categories (getCategories(), hasCategory())
3. Data URIs (getData(), getScheme(), getHost(), getPath())
4. MIME types (getType(), resolveType())
5. Extras (getExtras(), getStringExtra(), getIntExtra(), etc.)
6. Flags (getFlags(), addFlags())

Return a JSON object with the following schema:
{{
    \"params\": [
        {{
            \"name\": \"param_name\",
            \"type\": \"param_type\",
            \"value\": \"param_value\",
            \"flag\": \"-a/-c/-d/-t/-e/-f\"
        }}
    ],
    \"confidence\": 0.95
}}

Code to analyze:
{}",
        context
    );

    let request_body = json!({
        "model": config.model_type,
        "messages": [
            {
                "role": "system",
                "content": "You are an expert in Android development and ADB commands. Your task is to analyze Intent code and extract all possible parameters for ADB commands. Focus on finding all intent-related code patterns and their corresponding ADB parameters. Always respond with a valid JSON object containing 'params' array with parameter details and 'confidence' number. Do not include any other text or explanation."
            },
            {
                "role": "user",
                "content": prompt
            }
        ],
        "temperature": 0.3,
        "max_tokens": 4096,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "params": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": {
                                        "type": "string",
                                        "description": "The name of the parameter (e.g., action, category, data, type, extra)"
                                    },
                                    "type": {
                                        "type": "string",
                                        "description": "The type of the parameter (e.g., String, Integer, Boolean, Uri)"
                                    },
                                    "value": {
                                        "type": "string",
                                        "description": "The value for the parameter (for data URI, use the full URI string)"
                                    },
                                    "flag": {
                                        "type": "string",
                                        "description": "The ADB flag for the parameter (-a for action, -c for category, -d for data URI, -t for MIME type, -e for extra, -f for flag)",
                                        "enum": ["-a", "-c", "-d", "-t", "-e", "-f"]
                                    }
                                },
                                "required": ["name", "type", "value", "flag"]
                            }
                        },
                        "confidence": {
                            "type": "number",
                            "minimum": 0,
                            "maximum": 1
                        }
                    },
                    "required": ["params", "confidence"]
                }
            }
        }
    });

    let response = client
        .post(&format!("{}/chat/completions", config.api_url))
        .headers(headers)
        .json(&request_body)
        .send()
        .await
        .context("Failed to send request to LLM API")?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("LLM API error: {}", error_text));
    }

    let response_json: Value = response.json().await?;
    
    let content = response_json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            let response_str = serde_json::to_string_pretty(&response_json)
                .unwrap_or_else(|_| "Failed to format response".to_string());
            error!("Invalid response format. Full response: {}", response_str);
            anyhow::anyhow!("Invalid response format: missing content field")
        })?;

    let analysis: Value = serde_json::from_str(content)
        .context("Failed to parse LLM response as JSON")?;
    
    Ok(analysis)
}

fn parse_llm_response(analysis: &Value) -> Result<Vec<IntentParameter>> {
    let params = analysis["params"]
        .as_array()
        .ok_or_else(|| {
            let analysis_str = serde_json::to_string_pretty(analysis)
                .unwrap_or_else(|_| "Failed to format analysis".to_string());
            error!("Invalid params format in analysis: {}", analysis_str);
            anyhow::anyhow!("Invalid params format")
        })?
        .iter()
        .filter_map(|v| {
            let param = v.as_object()?;
            
            // 필수 필드 검증
            let name = param["name"].as_str()?;
            let param_type = param["type"].as_str()?;
            let flag = param["flag"].as_str()?;
            
            // flag 값 검증
            if !["-a", "-c", "-d", "-t", "-e", "-f"].contains(&flag) {
                error!("Invalid flag value: {} for parameter: {}", flag, name);
                return None;
            }
            
            // value가 있는 경우 사용, 없는 경우 기본값 생성
            let value = if let Some(v) = param["value"].as_str() {
                v.to_string()
            } else {
                // value가 없는 경우에만 기본값 생성
                match flag {
                    "-a" => format!("android.intent.action.{}", name),
                    "-c" => format!("android.intent.category.{}", name),
                    "-d" => format!("content://{}/{}", name, "example"),
                    "-t" => format!("{}/{}", name, "example"),
                    "-e" => match param_type.to_lowercase().as_str() {
                        "string" => "example_string".to_string(),
                        "integer" => "1".to_string(),
                        "boolean" => "true".to_string(),
                        "long" => "1".to_string(),
                        "float" => "1.0".to_string(),
                        "double" => "1.0".to_string(),
                        "uri" => format!("content://{}/{}", name, "example"),
                        _ => "example".to_string()
                    },
                    "-f" => "1".to_string(),
                    _ => "example".to_string()
                }
            };
            
            Some(IntentParameter {
                name: name.to_string(),
                param_type: param_type.to_string(),
                value,
                flag: flag.to_string(),
            })
        })
        .collect::<Vec<IntentParameter>>();

    if params.is_empty() {
        warn!("No valid parameters found in LLM response");
    } else {
        info!("Successfully parsed {} parameters from LLM response", params.len());
    }

    Ok(params)
}

fn validate_param_value(flag: &str, _value: &str, param_type: &str) -> bool {
    // value는 임의로 지정 가능하므로 항상 true 반환
    true
}

pub fn validate_adb_command(params: &[IntentParameter]) -> Result<()> {
    let mut has_action = false;
    let mut warnings: Vec<String> = Vec::new();
    
    // 파라미터가 비어있는 경우
    if params.is_empty() {
        warn!("No parameters provided for ADB command");
        return Ok(());
    }
    
    for param in params {
        match param.flag.as_str() {
            "-a" => {
                has_action = true;
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Action parameter '{}' has empty value", param.name));
                }
            },
            "-c" => {
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Category parameter '{}' has empty value", param.name));
                }
            },
            "-d" => {
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Data parameter '{}' has empty value", param.name));
                }
            },
            "-t" => {
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Type parameter '{}' has empty value", param.name));
                }
            },
            "-e" => {
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Extra parameter '{}' has empty value", param.name));
                }
            },
            "-f" => {
                if param.value.is_empty() {
                    warnings.push(format!("Warning: Flag parameter '{}' has empty value", param.name));
                }
            },
            _ => warnings.push(format!("Warning: Unknown flag '{}' for parameter '{}'", param.flag, param.name))
        }
    }
    
    // 최소한의 검증: action이 있어야 함
    if !has_action {
        warn!("ADB command must have at least one action (-a)");
        return Ok(());
    }
    
    // 경고 메시지가 있으면 출력
    if !warnings.is_empty() {
        warn!("{}", warnings.join("\n"));
    }
    
    Ok(())
}

pub fn find_source_file(component: &Component, _source_dir: &str) -> Result<PathBuf> {
    info!("Looking for source file for component: {}", component.name);
    
    // 컴포넌트의 manifest_dir 사용
    let manifest_dir = &component.manifest_dir;
    info!("Searching in directory: {}", manifest_dir.display());

    // 컴포넌트의 class_name과 package_name 사용
    let class_name = &component.class_name;
    let package_name = &component.package;
    
    info!("Searching for class: {} in package: {}", class_name, package_name);

    // AndroidManifest.xml이 있는 디렉토리에서 모든 Java/Kotlin 파일 검색
    let mut source_files = Vec::new();
    for entry in walkdir::WalkDir::new(manifest_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            if let Some(ext) = e.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                ext_str == "java" || ext_str == "kt"
            } else {
                false
            }
        }) {
        source_files.push(entry.path().to_path_buf());
    }

    info!("Found {} Java/Kotlin files to search", source_files.len());

    // 먼저 파일 이름으로 매칭 시도 (컴포넌트 클래스명과 파일명 일치)
    for file_path in &source_files {
        if let Some(file_name) = file_path.file_stem() {
            let file_name_str = file_name.to_string_lossy();
            if file_name_str.as_ref() == class_name.as_str() {
                info!("Found source file: {} (matched filename)", file_path.display());
                return Ok(file_path.clone());
            }
        }
    }

    // 파일 내용에서 컴포넌트 이름 검색
    for file_path in source_files {
        if let Ok(file) = File::open(&file_path) {
            let reader = BufReader::new(file);
            let mut content = String::new();
            for line in reader.lines() {
                if let Ok(line) = line {
                    content.push_str(&line);
                    content.push('\n');
                }
            }

            // 다양한 패턴으로 매칭 시도
            let is_match = content.contains(&component.name) || // 전체 이름
                content.contains(&format!("class {}", class_name)) || // 클래스 선언
                content.contains(&format!("class {} ", class_name)) || // 클래스 선언 (공백 포함)
                content.contains(&format!("extends {}", class_name)) || // 상속
                content.contains(&format!("implements {}", class_name)) || // 인터페이스 구현
                content.contains(&format!("new {}", class_name)) || // 인스턴스 생성
                content.contains(&format!("{}(", class_name)) || // 생성자 호출
                content.contains(&format!("package {}", package_name)); // 패키지 선언

            if is_match {
                info!("Found source file: {} (matched pattern)", file_path.display());
                return Ok(file_path);
            }
        }
    }
    
    Err(anyhow::anyhow!("Source file not found for component: {}", component.name))
}

async fn analyze_with_llm(context: &str, config: &LLMConfig) -> Result<IntentAnalysis, Box<dyn std::error::Error>> {
    let client = Client::new();
    
    let prompt = format!(
        "Analyze the following Android Intent code and suggest possible parameters for ADB command. Return a JSON object with the following schema:\n\n{{\"params\": [{{\"name\": \"param_name\", \"type\": \"param_type\", \"value\": \"param_value\", \"flag\": \"-a/-c/-e\"}}], \"confidence\": 0.95}}\n\nCode to analyze:\n{}",
        context
    );
    
    let response = client
        .post(&format!("{}/chat/completions", config.api_url))
        .json(&json!({
            "model": config.model_type,
            "messages": [
                {
                    "role": "system",
                    "content": "You are an expert in Android development and Intent analysis."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.7,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "schema": {
                        "type": "object",
                        "properties": {
                            "params": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": {
                                            "type": "string",
                                            "description": "The name of the parameter"
                                        },
                                        "type": {
                                            "type": "string",
                                            "description": "The type of the parameter (e.g., String, Integer, Boolean, Uri)"
                                        },
                                        "value": {
                                            "type": "string",
                                            "description": "The value or example value for the parameter"
                                        },
                                        "flag": {
                                            "type": "string",
                                            "description": "The ADB flag for the parameter (-a for action, -c for category, -e for extra)",
                                            "enum": ["-a", "-c", "-e"]
                                        }
                                    },
                                    "required": ["name", "type", "value", "flag"]
                                }
                            },
                            "confidence": {
                                "type": "number",
                                "minimum": 0,
                                "maximum": 1
                            }
                        },
                        "required": ["params", "confidence"]
                    }
                }
            }
        }))
        .send()
        .await?;
    
    let response_json: serde_json::Value = response.json().await?;
    
    // LLM 응답 파싱
    let content = response_json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("Invalid LLM response")?;
    
    let analysis: serde_json::Value = serde_json::from_str(content)?;
    
    let params = analysis["params"]
        .as_array()
        .ok_or("Invalid params format")?
        .iter()
        .filter_map(|v| {
            let param = v.as_object()?;
            let is_parcelable = param["is_parcelable"].as_bool().unwrap_or(false);
            
            if is_parcelable {
                // Parcelable인 경우 대체 파라미터들을 사용
                if let Some(alt_params) = param["alternative_params"].as_array() {
                    Some(alt_params.iter().filter_map(|alt_param| {
                        let alt = alt_param.as_object()?;
                        Some(IntentParameter {
                            name: alt["name"].as_str()?.to_string(),
                            param_type: alt["type"].as_str()?.to_string(),
                            value: alt["value"].as_str()?.to_string(),
                            flag: alt["flag"].as_str()?.to_string(),
                        })
                    }).collect::<Vec<IntentParameter>>())
                } else {
                    Some(Vec::new())
                }
            } else {
                // 일반 파라미터인 경우
                Some(vec![IntentParameter {
                    name: param["name"].as_str()?.to_string(),
                    param_type: param["type"].as_str()?.to_string(),
                    value: param["value"].as_str()?.to_string(),
                    flag: param["flag"].as_str()?.to_string(),
                }])
            }
        })
        .flatten()
        .collect::<Vec<IntentParameter>>();
    
    Ok(IntentAnalysis {
        intent_params: params,
        confidence: analysis["confidence"]
            .as_f64()
            .ok_or("Invalid confidence format")?,
        source_context: context.to_string(),
    })
}

pub fn generate_basic_params(component: &Component) -> Vec<IntentParameter> {
    let mut params = Vec::new();

    // Add action if available
    if let Some(action) = component.actions.iter().next() {
        params.push(IntentParameter {
            name: "action".to_string(),
            param_type: "String".to_string(),
            value: action.clone(),
            flag: "-a".to_string(),
        });
    }

    // Add category if available
    if let Some(category) = component.categories.iter().next() {
        params.push(IntentParameter {
            name: "category".to_string(),
            param_type: "String".to_string(),
            value: category.clone(),
            flag: "-c".to_string(),
        });
    }

    // Add data URI if scheme and host are available
    if !component.data_schemes.is_empty() && !component.data_hosts.is_empty() {
        let scheme = component.data_schemes.iter().next().unwrap();
        let host = component.data_hosts.iter().next().unwrap();
        let empty_path = String::new();
        let path = component.data_paths.iter().next().unwrap_or(&empty_path);
        
        let uri = if !path.is_empty() {
            format!("{}://{}{}", scheme, host, path)
        } else {
            format!("{}://{}", scheme, host)
        };

        params.push(IntentParameter {
            name: "data".to_string(),
            param_type: "Uri".to_string(),
            value: uri,
            flag: "-d".to_string(),
        });
    }

    // Add MIME type if available
    if let Some(mime_type) = component.data_mimeTypes.iter().next() {
        params.push(IntentParameter {
            name: "type".to_string(),
            param_type: "String".to_string(),
            value: mime_type.clone(),
            flag: "-t".to_string(),
        });
    }

    params
}

pub fn convert_to_intent_parameters(params: &[IntentParameter]) -> Vec<IntentParameter> {
    params.to_vec()
} 