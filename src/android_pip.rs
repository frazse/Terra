//! Android Picture-in-Picture (PiP) bridge between JS (`terra-ui.js`) and `MainActivity`.

#[cfg(target_os = "android")]
mod jni_pip {
    use crate::ble::{alog, find_app_class, get_java_vm};
    use jni::objects::JValue;

    pub fn get_status_json() -> Result<String, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let res = env
            .call_static_method(&cls, "getPipStatusJson", "()Ljava/lang/String;", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call getPipStatusJson: {e}");
                alog(&msg);
                msg
            })?;
        let obj = res.l().map_err(|e| format!("result getPipStatusJson: {e}"))?;
        if obj.is_null() {
            return Err("getPipStatusJson returned null".to_string());
        }
        let jstr = jni::objects::JString::from(obj);
        let s: String = env.get_string(&jstr).map_err(|e| e.to_string())?.into();
        Ok(s)
    }

    pub fn configure(
        enabled: bool,
        auto_enter: bool,
        hud_mode: &str,
        show_button: bool,
        in_ride: bool,
    ) -> Result<String, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let j_mode = env
            .new_string(hud_mode)
            .map_err(|e| format!("new_string: {e}"))?;
        let res = env
            .call_static_method(
                &cls,
                "configurePip",
                "(ZZLjava/lang/String;ZZ)Ljava/lang/String;",
                &[
                    JValue::Bool(u8::from(enabled)),
                    JValue::Bool(u8::from(auto_enter)),
                    JValue::Object(&j_mode),
                    JValue::Bool(u8::from(show_button)),
                    JValue::Bool(u8::from(in_ride)),
                ],
            )
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call configurePip: {e}");
                alog(&msg);
                msg
            })?;
        let obj = res.l().map_err(|e| format!("result configurePip: {e}"))?;
        if obj.is_null() {
            return Err("configurePip returned null".to_string());
        }
        let jstr = jni::objects::JString::from(obj);
        let s: String = env.get_string(&jstr).map_err(|e| e.to_string())?.into();
        Ok(s)
    }

    pub fn enter_pip() -> Result<bool, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let res = env
            .call_static_method(&cls, "enterPipModeJni", "()Z", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call enterPipModeJni: {e}");
                alog(&msg);
                msg
            })?;
        res.z().map_err(|e| format!("result enterPipModeJni: {e}"))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_pip_status() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let raw = jni_pip::get_status_json()?;
        serde_json::from_str(&raw).map_err(|e| format!("invalid pip status json: {e}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(serde_json::json!({ "supported": false }))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_pip_configure(
    enabled: Option<bool>,
    auto_enter: Option<bool>,
    hud_mode: Option<String>,
    show_button: Option<bool>,
    in_ride: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let current = terra_pip_status().unwrap_or_else(|_| serde_json::json!({}));
        let cur_enabled = current
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cur_auto = current
            .get("autoEnter")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let cur_btn = current
            .get("showButton")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cur_ride = current
            .get("inRide")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let _ = hud_mode;
        let next_mode = "compact";

        let raw = jni_pip::configure(
            enabled.unwrap_or(cur_enabled),
            auto_enter.unwrap_or(cur_auto),
            next_mode,
            show_button.unwrap_or(cur_btn),
            in_ride.unwrap_or(cur_ride),
        )?;
        serde_json::from_str(&raw).map_err(|e| format!("invalid pip config json: {e}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (enabled, auto_enter, hud_mode, show_button, in_ride);
        Ok(serde_json::json!({ "supported": false }))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_enter_pip() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let entered = jni_pip::enter_pip()?;
        Ok(serde_json::json!({ "entered": entered }))
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(serde_json::json!({ "entered": false }))
    }
}
