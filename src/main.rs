use std::path::PathBuf;
use std::process::Command;
use std::collections::HashMap;
use clap::Parser;
use crate::manifest::{Component, find_manifest_files, parse_manifest};
use crate::permissions::get_permission_protection_level;
use crate::utils::adb::ADBCommand;
use crate::llm::{LLMConfig, fetch_available_models};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Result, Context};
use tracing::{info, error, warn, Level};
use tracing_subscriber::FmtSubscriber;

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

struct SourceFileCache {
    files: HashMap<String, Vec<PathBuf>>,
    manifest_dir: PathBuf,
}

impl SourceFileCache {
    fn new(manifest_path: &PathBuf) -> Self {
        Self {
            files: HashMap::new(),
            manifest_dir: manifest_path.parent().unwrap().to_path_buf(),
        }
    }

    fn scan_directory(&mut self, dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
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

    fn find_component_file(&self, component: &Component) -> Option<PathBuf> {
        let component_name = component.name.split('.').last().unwrap_or(&component.name);
        
        // 1. 정확한 이름 매칭
        if let Some(files) = self.files.get(component_name) {
            if files.len() == 1 {
                return Some(files[0].clone());
            }
        }

        // 2. 부분 이름 매칭
        let mut matches = Vec::new();
        for (name, files) in &self.files {
            if name.contains(component_name) || component_name.contains(name) {
                matches.extend(files.clone());
            }
        }

        // 매니페스트 디렉토리 하위에 있는 파일만 필터링
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

fn find_source_dir(manifest_path: &PathBuf) -> Option<PathBuf> {
    let mut cache = SourceFileCache::new(manifest_path);
    let manifest_dir = cache.manifest_dir.clone();
    
    // 매니페스트 디렉토리와 그 하위 디렉토리 스캔
    if let Err(e) = cache.scan_directory(&manifest_dir) {
        eprintln!("소스 파일 스캔 중 오류 발생: {}", e);
        return None;
    }

    // 스캔된 파일이 있는지 확인
    if cache.files.is_empty() {
        return None;
    }

    // 첫 번째 파일의 부모 디렉토리 반환
    cache.files.values().next()
        .and_then(|files| files.first())
        .and_then(|path| path.parent())
        .map(|p| p.to_path_buf())
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
    let llm_config = setup_llm_config(&args)?;
    
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

fn setup_llm_config(args: &Args) -> Result<LLMConfig> {
    let config = LLMConfig::new(
        args.llm_url.clone().unwrap_or_else(|| "http://localhost:1234/v1".to_string()),
        args.llm_key.clone(),
        args.llm_model.clone().unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
    );
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

    // Try to find and analyze source file
    match llm::analyzer::find_source_file(component, "") {
        Ok(source_file) => {
            // Source file found, use LLM analysis
            match llm::analyzer::analyze_intent(component, &source_file.to_string_lossy(), llm_config).await {
                Ok(analysis) => {
                    adb_cmd.set_intent_params(&analysis.intent_params);
                }
                Err(e) => {
                    warn!("Failed to analyze intent with LLM: {}. Using basic parameters.", e);
                    let basic_params = llm::analyzer::generate_basic_params(component);
                    adb_cmd.set_intent_params(&basic_params);
                }
            }
        }
        Err(_) => {
            // Source file not found, use basic parameters
            info!("Source file not found for {}. Using basic parameters from manifest.", component.name);
            let basic_params = llm::analyzer::generate_basic_params(component);
            adb_cmd.set_intent_params(&basic_params);
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
    
    // sharedUserId가 있는 경우 표시
    if let Some(shared_user_id) = &component.shared_user_id {
        println!("\x1b[1;35mNote: This component has sharedUserId: {}\x1b[0m", shared_user_id);
    }
    
    Ok(())
}
