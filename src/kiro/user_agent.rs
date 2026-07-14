//! Kiro 请求使用的 User-Agent 构造工具。
//!
//! 统一维护 runtime / management 相关的 SDK 版本和 UA 格式，
//! 避免各调用点散落硬编码字符串。

use crate::model::config::Config;

pub const MANAGEMENT_SDK_VERSION: &str = "1.0.0";
/// Kiro IDE 1.0.138+ runtime 使用 aws-sdk-js/1.0.0 + api/kiroruntime
pub const RUNTIME_STREAMING_SDK_VERSION: &str = "1.0.0";

fn kiro_ide_label(config: &Config, machine_id: &str) -> String {
    format!("KiroIDE-{}-{}", config.kiro_version, machine_id)
}

fn build_x_amz_user_agent(sdk_version: &str, config: &Config, machine_id: &str) -> String {
    format!(
        "aws-sdk-js/{} {}",
        sdk_version,
        kiro_ide_label(config, machine_id)
    )
}

fn build_user_agent(
    sdk_version: &str,
    config: &Config,
    machine_id: &str,
    api_name: &str,
    metrics: &str,
) -> String {
    format!(
        "aws-sdk-js/{} ua/2.1 os/{} lang/js md/nodejs#{} api/{}#{} {} {}",
        sdk_version,
        config.system_version,
        config.node_version,
        api_name,
        sdk_version,
        metrics,
        kiro_ide_label(config, machine_id)
    )
}

pub fn runtime_streaming_x_amz_user_agent(config: &Config, machine_id: &str) -> String {
    build_x_amz_user_agent(RUNTIME_STREAMING_SDK_VERSION, config, machine_id)
}

pub fn runtime_streaming_user_agent(config: &Config, machine_id: &str) -> String {
    build_user_agent(
        RUNTIME_STREAMING_SDK_VERSION,
        config,
        machine_id,
        "kiroruntime",
        "m/N",
    )
}

pub fn management_runtime_x_amz_user_agent(config: &Config, machine_id: &str) -> String {
    build_x_amz_user_agent(MANAGEMENT_SDK_VERSION, config, machine_id)
}

pub fn management_runtime_user_agent(config: &Config, machine_id: &str) -> String {
    build_user_agent(
        MANAGEMENT_SDK_VERSION,
        config,
        machine_id,
        "codewhispererruntime",
        "m/N,E",
    )
}

pub fn management_control_plane_x_amz_user_agent(config: &Config, machine_id: &str) -> String {
    build_x_amz_user_agent(MANAGEMENT_SDK_VERSION, config, machine_id)
}

pub fn management_control_plane_user_agent(config: &Config, machine_id: &str) -> String {
    build_user_agent(
        MANAGEMENT_SDK_VERSION,
        config,
        machine_id,
        "kirocontrolplanebearer",
        "m/N,E",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let mut config = Config::default();
        config.kiro_version = "1.0.138".to_string();
        config.system_version = "win32#10.0.19045".to_string();
        config.node_version = "22.22.0".to_string();
        config
    }

    #[test]
    fn test_runtime_streaming_user_agent() {
        let config = test_config();
        let ua = runtime_streaming_user_agent(&config, "machine123");
        assert_eq!(
            ua,
            "aws-sdk-js/1.0.0 ua/2.1 os/win32#10.0.19045 lang/js md/nodejs#22.22.0 api/kiroruntime#1.0.0 m/N KiroIDE-1.0.138-machine123"
        );
    }

    #[test]
    fn test_management_control_plane_x_amz_user_agent() {
        let config = test_config();
        let ua = management_control_plane_x_amz_user_agent(&config, "machine123");
        assert_eq!(ua, "aws-sdk-js/1.0.0 KiroIDE-1.0.138-machine123");
    }
}
