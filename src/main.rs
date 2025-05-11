use std::path::PathBuf;
use std::process::Command;
use clap::Parser;
use crate::manifest::{Component, find_manifest_files, parse_manifest};
use crate::permissions::get_permission_protection_level;
use crate::utils::adb::ADBCommand;
use crate::utils::source::{find_source_file, parse_intent_parameters, intent_parameters_to_adb_args};
use crate::llm::{LLMConfig, fetch_available_models};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Result, Context};
use tracing::{info, error, warn, Level};
use tracing_subscriber::FmtSubscriber;
use tree_sitter::Parser as TreeSitterParser;
mod manifest;
mod permissions;
mod utils;
mod llm;

/// Android 프로젝트에서 AndroidManifest.xml 파일을 검색하고 exported 컴포넌트를 파싱하는 프로그램
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 검색할 디렉토리 경로
    #[arg(short, long)]
    dir: String,

    /// 패키지 이름 (선택)
    #[arg(short, long)]
    package: Option<String>,

    /// 최대 권한 보호 수준 (normal, dangerous, signature)
    #[arg(short, long, default_value = "signature")]
    max_permission_level: String,

    /// 현재 설치된 패키지의 컴포넌트만 표시
    #[arg(short, long)]
    alive_only: bool,

    /// sharedUserId가 있는 컴포넌트 제외
    #[arg(long)]
    no_shared_userid: bool,

    /// LLM API URL (로컬 LLM의 경우 기본값: http://localhost:1234/v1)
    #[arg(long)]
    llm_url: Option<String>,

    /// LLM API 키 (선택사항)
    #[arg(long)]
    llm_key: Option<String>,

    /// LLM 모델 (모델 이름 또는 번호)
    #[arg(long)]
    llm_model: Option<String>,

    /// 로그 레벨
    #[arg(long, default_value = "info")]
    log_level: String,
}

fn get_permission_level_value(level: &str) -> u8 {
    match level {
        "normal" => 1,
        "dangerous" => 2,
        "signature" => 3,
        "signature|privileged" => 4,
        _ => 0,
    }
}

fn should_show_component(component: &Component, max_level: &str) -> bool {
    let max_level_value = get_permission_level_value(max_level);
    
    // 컴포넌트의 권한들 중 가장 높은 수준 확인
    let mut highest_level = 0;
    
    for permission in &component.permissions {
        let level = get_permission_level_value(get_permission_protection_level(permission));
        highest_level = highest_level.max(level);
    }
    
    for permission in &component.intent_filter_permissions {
        let level = get_permission_level_value(get_permission_protection_level(permission));
        highest_level = highest_level.max(level);
    }
    
    // 권한이 없는 경우 normal로 간주
    if highest_level == 0 {
        highest_level = 1;
    }
    
    highest_level <= max_level_value
}

fn get_alive_packages() -> Result<Vec<String>> {
    let output = Command::new("adb")
        .args(["shell", "pm", "list", "packages"])
        .output()
        .context("Failed to execute adb command")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("Failed to execute adb command"));
    }

    let stdout = String::from_utf8(output.stdout)
        .context("Failed to parse adb command output")?;
    
    let packages: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            if line.starts_with("package:") {
                Some(line[8..].to_string())
            } else {
                None
            }
        })
        .collect();

    Ok(packages)
}

async fn select_model(api_url: &str, api_key: Option<&str>, model_arg: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    println!("사용 가능한 모델을 가져오는 중...");
    let models = fetch_available_models(api_url, api_key).await?;
    
    if models.is_empty() {
        return Err("사용 가능한 모델이 없습니다.".into());
    }
    
    println!("\n사용 가능한 모델:");
    for (i, model) in models.iter().enumerate() {
        println!("{}. {}", i + 1, model);
    }

    // 모델 인자가 있는 경우
    if let Some(model) = model_arg {
        // 숫자로 입력된 경우
        if let Ok(index) = model.parse::<usize>() {
            if index > 0 && index <= models.len() {
                return Ok(models[index - 1].clone());
            }
        }
        
        // 문자열로 입력된 경우 (부분 일치 검색)
        if let Some(found_model) = models.iter()
            .find(|m| m.to_lowercase().contains(&model.to_lowercase())) {
            return Ok(found_model.clone());
        }
        
        println!("지정된 모델을 찾을 수 없습니다. 대화형 선택으로 전환합니다.");
    }
    
    // 대화형 선택
    loop {
        println!("\n모델 번호를 선택하세요 (1-{}): ", models.len());
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if let Ok(index) = input.trim().parse::<usize>() {
            if index > 0 && index <= models.len() {
                return Ok(models[index - 1].clone());
            }
        }
        println!("잘못된 선택입니다. 다시 시도하세요.");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // 로깅 설정
    setup_logging(&args.log_level)?;
    
    // LLM 설정
    let llm_config = setup_llm_config(&args).await?;
    
    // 매니페스트 파서 설정
    let manifest_dir = setup_manifest_parser(&args)?;
    
    // 컴포넌트 분석
    let components = analyze_components(&manifest_dir, &args).await?;
    
    // ADB 명령어 생성 및 실행
    generate_and_run_adb_commands(&components, &llm_config).await?;

    Ok(())
}

fn setup_logging(log_level: &str) -> Result<()> {
    let level = match log_level.to_lowercase().as_str() {
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let _subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_names(false)
        .with_ansi(true)
        .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339())
        .with_level(true)
        .init();

    Ok(())
}

async fn setup_llm_config(args: &Args) -> Result<LLMConfig> {
    let config = match &args.llm_url {
        None => {
            // LLM URL이 없는 경우 빈 설정 반환
            LLMConfig::new(
                String::new(),
                None,
                String::new(),
            )
        }
        Some(url) => {
            // LLM URL이 있는 경우 모델 선택
            let model = if let Some(model) = &args.llm_model {
                model.clone()
            } else {
                // 모델이 지정되지 않은 경우 인터랙티브 선택
                select_model(url, args.llm_key.as_deref(), None)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to select model: {}", e))?
            };

            LLMConfig::new(
                url.clone(),
                args.llm_key.clone(),
                model,
            )
        }
    };
    Ok(config)
}

fn setup_manifest_parser(args: &Args) -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(&args.dir);
    Ok(manifest_dir)
}

async fn analyze_components(manifest_dir: &PathBuf, args: &Args) -> Result<Vec<Component>> {
    info!("Scanning directory for AndroidManifest.xml files: {}", manifest_dir.display());
    
    // Find all AndroidManifest.xml files
    let manifest_files = find_manifest_files(manifest_dir.to_str().unwrap());
    info!("Found {} AndroidManifest.xml files", manifest_files.len());

    let mut all_components = Vec::new();
    
    // Parse each manifest file
    for manifest_path in manifest_files {
        info!("Parsing manifest file: {}", manifest_path.display());
        match parse_manifest(&manifest_path, args.package.as_deref()) {
            Ok(components) => {
                info!("Found {} components in {}", components.len(), manifest_path.display());
                all_components.extend(components);
            }
            Err(e) => {
                error!("Failed to parse manifest file {}: {}", manifest_path.display(), e);
            }
        }
    }

    // Filter components based on various criteria
    let components: Vec<Component> = all_components.into_iter()
        .filter(|component| {
            // Filter by package if alive_only is set
            if args.alive_only {
                let alive_packages = get_alive_packages().unwrap_or_default();
                if !alive_packages.contains(&component.package) {
                    return false;
                }
            }

            // Filter out components with sharedUserId if no_shared_userid is set
            if args.no_shared_userid && component.shared_user_id.is_some() {
                return false;
            }

            true
        })
        .collect();

    info!("Found {} components to analyze", components.len());
    Ok(components)
}

async fn generate_and_run_adb_commands(
    components: &[Component],
    llm_config: &LLMConfig,
) -> Result<()> {
    let adb = Arc::new(Mutex::new(ADBCommand::new()?));
    
    for component in components {
        match generate_adb_command(component, llm_config, &adb).await {
            Ok(_) => info!("Successfully generated ADB command for {}", component.name),
            Err(e) => error!("Failed to generate ADB command for {}: {}", component.name, e),
        }
    }

    Ok(())
}

async fn generate_adb_command(
    component: &Component,
    llm_config: &LLMConfig,
    adb: &Arc<Mutex<ADBCommand>>,
) -> Result<()> {
    let mut adb_cmd = adb.lock().await;
    adb_cmd.set_component(component);
    info!("Component: {}", component.name);

    // LLM URL이 지정되지 않은 경우 기본 파라미터만 사용
    if llm_config.api_url.is_empty() {
        info!("LLM URL not provided. Using basic parameters from manifest.");
        match find_source_file(component, "") {
            Ok(source_file) => {
                // Parse intent parameters from source code
                match parse_intent_parameters(&source_file) {
                    Ok(parameters) => {
                        info!("Found {} intent parameters in source code", parameters.len());
                        let adb_args = intent_parameters_to_adb_args(&parameters);
                        for arg in adb_args {
                            adb_cmd.add_extra_arg(&arg);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse intent parameters: {}. Using basic parameters.", e);
                        let basic_params = llm::analyzer::generate_basic_params(component);
                        llm::analyzer::validate_adb_command(&basic_params)
                            .context(format!("Failed to validate basic parameters for component: {}", component.name))?;
                        adb_cmd.set_intent_params(&basic_params);
                    }
                }
            }
            Err(e) => {
                warn!("Could not find source file: {}. Using basic parameters.", e);
                let basic_params = llm::analyzer::generate_basic_params(component);
                llm::analyzer::validate_adb_command(&basic_params)
                    .context(format!("Failed to validate basic parameters for component: {}", component.name))?;
                adb_cmd.set_intent_params(&basic_params);
            }
        }
    } else {
        // Try to find and analyze source file
        match find_source_file(component, "") {
            Ok(source_file) => {
                // First try to parse intent parameters from source code
                if let Ok(parameters) = parse_intent_parameters(&source_file) {
                    info!("Found {} intent parameters in source code", parameters.len());
                    let adb_args = intent_parameters_to_adb_args(&parameters);
                    for arg in adb_args {
                        adb_cmd.add_extra_arg(&arg);
                    }
                } else {
                    // If parsing fails, fall back to LLM analysis
                    match llm::analyzer::analyze_intent(component, &source_file.to_string_lossy(), llm_config).await {
                        Ok(analysis) => {
                            llm::analyzer::validate_adb_command(&analysis.intent_params)
                                .context(format!(
                                    "Failed to validate LLM analysis parameters for component: {}\nSource file: {}\nComponent type: {}\nPackage: {}",
                                    component.name,
                                    source_file.display(),
                                    component.component_type,
                                    component.package
                                ))?;
                            adb_cmd.set_intent_params(&analysis.intent_params);
                        }
                        Err(e) => {
                            warn!("Failed to analyze intent with LLM: {}. Using basic parameters.", e);
                            let basic_params = llm::analyzer::generate_basic_params(component);
                            llm::analyzer::validate_adb_command(&basic_params)
                                .context(format!("Failed to validate basic parameters for component: {}", component.name))?;
                            adb_cmd.set_intent_params(&basic_params);
                        }
                    }
                }
            }
            Err(e) => {
                // Source file not found, use basic parameters
                warn!("Could not find source file: {}. Using basic parameters.", e);
                let basic_params = llm::analyzer::generate_basic_params(component);
                llm::analyzer::validate_adb_command(&basic_params)
                    .context(format!("Failed to validate basic parameters for component: {}", component.name))?;
                adb_cmd.set_intent_params(&basic_params);
            }
        }
    }
    
    let command = adb_cmd.build_command()
        .context("Failed to build ADB command")?;
    
    // ADB 명령어를 특별한 형식으로 출력
    println!("\n\x1b[1;36mGenerated ADB command:\x1b[0m\n\x1b[1;33m{}\x1b[0m", command);
    
    // 매니페스트 정보 출력
    println!("\x1b[1;34mManifest: {}:{}\x1b[0m", 
        component.manifest_path.display(),
        component.manifest_line
    );
    if let Some(xml) = &component.xml_element {
        println!("\x1b[1;35mComponent XML:\x1b[0m\n{}", xml);
    }
    // Display source file information if available
    if let Ok(source_file) = find_source_file(component, "") {
        println!("\x1b[1;32mSource file: {}\x1b[0m", source_file.display());
    }

    // sharedUserId가 있는 경우 표시
    if let Some(shared_user_id) = &component.shared_user_id {
        println!("\x1b[1;35mNote: This component has sharedUserId: {}\x1b[0m", shared_user_id);
    }
    
    println!();
    Ok(())
}
