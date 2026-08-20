use crate::security::rules::{
    BuiltinRule, BuiltinRuleRepository, CustomRule, CustomRuleRepository,
    CreateCustomRuleInput, UpdateBuiltinRuleInput, seed_builtin_rules,
};
use crate::core::error::{CommandResult, CommandResultExt};
use crate::AppState;

#[tauri::command]
pub async fn get_builtin_security_rules(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<Vec<BuiltinRule>> {
    // Auto-seed if empty
    let rules = BuiltinRuleRepository::get_all(&state.db.pool)
        .await
        .command_error("SECURITY_RULE_LIST_FAILED", "读取内置安全规则失败", true)?;
    if rules.is_empty() {
        seed_builtin_rules(&state.db.pool)
            .await
            .command_error("SECURITY_RULE_SEED_FAILED", "初始化内置安全规则失败", false)?;
        return BuiltinRuleRepository::get_all(&state.db.pool)
            .await
            .command_error("SECURITY_RULE_LIST_FAILED", "读取内置安全规则失败", true);
    }
    Ok(rules)
}

#[tauri::command]
pub async fn update_builtin_security_rule(
    id: String,
    input: UpdateBuiltinRuleInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<()> {
    BuiltinRuleRepository::update(&state.db.pool, &id, &input)
        .await
        .command_error("SECURITY_RULE_UPDATE_FAILED", "更新内置安全规则失败", false)
}

#[tauri::command]
pub async fn delete_builtin_security_rule(
    id: String,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<()> {
    BuiltinRuleRepository::delete(&state.db.pool, &id)
        .await
        .command_error("SECURITY_RULE_DELETE_FAILED", "删除内置安全规则失败", false)
}

#[tauri::command]
pub async fn reset_builtin_security_rules(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<Vec<BuiltinRule>> {
    BuiltinRuleRepository::reset_to_defaults(&state.db.pool)
        .await
        .command_error("SECURITY_RULE_RESET_FAILED", "重置内置安全规则失败", false)?;
    BuiltinRuleRepository::get_all(&state.db.pool)
        .await
        .command_error("SECURITY_RULE_LIST_FAILED", "读取内置安全规则失败", true)
}

#[tauri::command]
pub async fn get_custom_security_rules(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<Vec<CustomRule>> {
    CustomRuleRepository::get_all(&state.db.pool)
        .await
        .command_error("CUSTOM_RULE_LIST_FAILED", "读取自定义安全规则失败", true)
}

#[tauri::command]
pub async fn create_custom_security_rule(
    input: CreateCustomRuleInput,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<CustomRule> {
    CustomRuleRepository::create(&state.db.pool, &input)
        .await
        .command_error("CUSTOM_RULE_CREATE_FAILED", "创建自定义安全规则失败", false)
}

#[tauri::command]
pub async fn toggle_custom_security_rule(
    id: String,
    enabled: bool,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<()> {
    CustomRuleRepository::update_enabled(&state.db.pool, &id, enabled)
        .await
        .command_error("CUSTOM_RULE_UPDATE_FAILED", "更新自定义安全规则失败", false)
}

#[tauri::command]
pub async fn delete_custom_security_rule(
    id: String,
    state: tauri::State<'_, std::sync::Arc<AppState>>,
) -> CommandResult<()> {
    CustomRuleRepository::delete(&state.db.pool, &id)
        .await
        .command_error("CUSTOM_RULE_DELETE_FAILED", "删除自定义安全规则失败", false)
}
