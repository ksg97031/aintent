# AIntent - Android Intent Analyzer

AIntent is a powerful tool for analyzing Android components and generating ADB commands to interact with them. It parses AndroidManifest.xml files, analyzes component declarations, and generates appropriate ADB commands with intent parameters.

## Features

- 🔍 **Manifest Analysis**: Parses AndroidManifest.xml files to extract component information
- 🎯 **Component Detection**: Identifies activities, services, receivers, and providers
- 🤖 **LLM Integration**: Uses Language Model to analyze source code and extract intent parameters
- 📱 **ADB Command Generation**: Generates ADB commands with proper intent parameters
- 🔒 **Permission Analysis**: Analyzes component permissions and protection levels
- 🎨 **Colorful Output**: Provides clear, color-coded output for better readability

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/aintent.git
cd aintent

# Build the project
cargo build --release
```

## Usage

```bash
# Basic usage
./target/release/aintent -d /path/to/android/project

# With package filter
./target/release/aintent -d /path/to/android/project -p com.example.app

# Show only components from installed packages
./target/release/aintent -d /path/to/android/project --alive-only

# Exclude components with sharedUserId
./target/release/aintent -d /path/to/android/project --no-shared-userid

# Use custom LLM configuration
./target/release/aintent -d /path/to/android/project --llm-url http://localhost:1234/v1 --llm-model gpt-3.5-turbo
```

### Command Line Options

- `-d, --dir`: Directory to search for AndroidManifest.xml files
- `-p, --package`: Filter components by package name
- `--max-permission-level`: Maximum permission protection level (normal, dangerous, signature)
- `--alive-only`: Show only components from installed packages
- `--no-shared-userid`: Exclude components with sharedUserId
- `--llm-url`: LLM API URL (default: http://localhost:1234/v1)
- `--llm-key`: LLM API key (optional)
- `--llm-model`: LLM model name or number
- `--log-level`: Logging level (debug, info, warn, error)

## Output Format

The tool provides color-coded output with the following information:

- 🔵 **Generated ADB command**: The complete ADB command to interact with the component
- 🟢 **Manifest location**: File path and line number where the component is declared
- 🟣 **Shared User ID**: Information about shared user ID if present

## Requirements

- Rust 1.70 or higher
- ADB (Android Debug Bridge) installed and configured
- LLM API access (optional, for advanced intent analysis)

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. 