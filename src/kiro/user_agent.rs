//! Kiro 请求使用的 User-Agent 构造工具。
//!
//! 统一维护 runtime / management 相关的 SDK 版本和 UA 格式，
//! 协议指纹（kiroVersion / systemVersion / nodeVersion）为程序内常量。

use crate::model::config::{KIRO_VERSION, NODE_VERSION, SYSTEM_VERSION};

pub const MANAGEMENT_SDK_VERSION: &str = "1.0.0";
/// Kiro IDE 1.0.138+ runtime 使用 aws-sdk-js/1.0.0 + api/kiroruntime
pub const RUNTIME_STREAMING_SDK_VERSION: &str = "1.0.0";

/// Kiro CLI / Amazon Q for CLI（抓包 2026-07-14，kiro-cli 2.12.1）
pub const CLI_RUST_SDK_VERSION: &str = "1.3.15";
pub const CLI_STREAMING_API_VERSION: &str = "0.1.17975";
pub const CLI_RUSTC_VERSION: &str = "1.92.0";
pub const CLI_APP_VERSION: &str = "2.12.1";

fn kiro_ide_label(machine_id: &str) -> String {
    format!("KiroIDE-{}-{}", KIRO_VERSION, machine_id)
}

fn build_x_amz_user_agent(sdk_version: &str, machine_id: &str) -> String {
    format!(
        "aws-sdk-js/{} {}",
        sdk_version,
        kiro_ide_label(machine_id)
    )
}

fn build_user_agent(sdk_version: &str, machine_id: &str, api_name: &str, metrics: &str) -> String {
    format!(
        "aws-sdk-js/{} ua/2.1 os/{} lang/js md/nodejs#{} api/{}#{} {} {}",
        sdk_version,
        SYSTEM_VERSION,
        NODE_VERSION,
        api_name,
        sdk_version,
        metrics,
        kiro_ide_label(machine_id)
    )
}

pub fn runtime_streaming_x_amz_user_agent(machine_id: &str) -> String {
    build_x_amz_user_agent(RUNTIME_STREAMING_SDK_VERSION, machine_id)
}

pub fn runtime_streaming_user_agent(machine_id: &str) -> String {
    build_user_agent(
        RUNTIME_STREAMING_SDK_VERSION,
        machine_id,
        "kiroruntime",
        "m/N",
    )
}

pub fn management_runtime_x_amz_user_agent(machine_id: &str) -> String {
    build_x_amz_user_agent(MANAGEMENT_SDK_VERSION, machine_id)
}

pub fn management_runtime_user_agent(machine_id: &str) -> String {
    build_user_agent(
        MANAGEMENT_SDK_VERSION,
        machine_id,
        "codewhispererruntime",
        "m/N,E",
    )
}

pub fn management_control_plane_x_amz_user_agent(machine_id: &str) -> String {
    build_x_amz_user_agent(MANAGEMENT_SDK_VERSION, machine_id)
}

pub fn management_control_plane_user_agent(machine_id: &str) -> String {
    build_user_agent(
        MANAGEMENT_SDK_VERSION,
        machine_id,
        "kirocontrolplanebearer",
        "m/N,E",
    )
}

/// CLI 的 os 段：官方为 `windows` / `linux` / `macos`，按编译目标选择。
pub fn cli_os_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Kiro CLI GenerateAssistantResponse 的 x-amz-user-agent
pub fn cli_streaming_x_amz_user_agent() -> String {
    format!(
        "aws-sdk-rust/{} ua/2.1 api/codewhispererstreaming/{} os/{} lang/rust/{} m/F app/AmazonQ-For-CLI",
        CLI_RUST_SDK_VERSION,
        CLI_STREAMING_API_VERSION,
        cli_os_label(),
        CLI_RUSTC_VERSION,
    )
}

/// Kiro CLI GenerateAssistantResponse 的 User-Agent
pub fn cli_streaming_user_agent() -> String {
    format!(
        "aws-sdk-rust/{} ua/2.1 api/codewhispererstreaming/{} os/{} lang/rust/{} md/appVersion-{} app/AmazonQ-For-CLI",
        CLI_RUST_SDK_VERSION,
        CLI_STREAMING_API_VERSION,
        cli_os_label(),
        CLI_RUSTC_VERSION,
        CLI_APP_VERSION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_streaming_user_agent() {
        let ua = runtime_streaming_user_agent("machine123");
        assert_eq!(
            ua,
            format!(
                "aws-sdk-js/1.0.0 ua/2.1 os/{} lang/js md/nodejs#{} api/kiroruntime#1.0.0 m/N KiroIDE-{}-machine123",
                SYSTEM_VERSION, NODE_VERSION, KIRO_VERSION
            )
        );
    }

    #[test]
    fn test_management_control_plane_x_amz_user_agent() {
        let ua = management_control_plane_x_amz_user_agent("machine123");
        assert_eq!(
            ua,
            format!("aws-sdk-js/1.0.0 KiroIDE-{}-machine123", KIRO_VERSION)
        );
    }

    #[test]
    fn test_cli_streaming_user_agent() {
        let ua = cli_streaming_user_agent();
        assert!(ua.contains("aws-sdk-rust/1.3.15"));
        assert!(ua.contains("app/AmazonQ-For-CLI"));
        assert!(ua.contains(&format!("os/{}", cli_os_label())));
        let xua = cli_streaming_x_amz_user_agent();
        assert!(xua.contains("m/F app/AmazonQ-For-CLI"));
    }

    #[test]
    fn test_cli_os_label_is_known() {
        assert!(matches!(cli_os_label(), "windows" | "macos" | "linux"));
    }
}
