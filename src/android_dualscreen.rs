//! Android Dual-Screen Presentation bridge between JS (`dualscreen.js` / `terra-ui.js`) and `MainActivity`.

#[cfg(target_os = "android")]
mod jni_dual {
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
            .call_static_method(&cls, "getDualScreenStatusJson", "()Ljava/lang/String;", &[])
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call getDualScreenStatusJson: {e}");
                alog(&msg);
                msg
            })?;
        let obj = res
            .l()
            .map_err(|e| format!("result getDualScreenStatusJson: {e}"))?;
        if obj.is_null() {
            return Err("getDualScreenStatusJson returned null".to_string());
        }
        let jstr = jni::objects::JString::from(obj);
        let s: String = env.get_string(&jstr).map_err(|e| e.to_string())?.into();
        Ok(s)
    }

    pub fn configure(
        enabled: bool,
        mode: &str,
        keep_nameplates: bool,
        in_ride: bool,
    ) -> Result<String, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let j_mode = env
            .new_string(mode)
            .map_err(|e| format!("new_string: {e}"))?;
        let res = env
            .call_static_method(
                &cls,
                "configureDualScreen",
                "(ZLjava/lang/String;ZZ)Ljava/lang/String;",
                &[
                    JValue::Bool(u8::from(enabled)),
                    JValue::Object(&j_mode),
                    JValue::Bool(u8::from(keep_nameplates)),
                    JValue::Bool(u8::from(in_ride)),
                ],
            )
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call configureDualScreen: {e}");
                alog(&msg);
                msg
            })?;
        let obj = res
            .l()
            .map_err(|e| format!("result configureDualScreen: {e}"))?;
        if obj.is_null() {
            return Err("configureDualScreen returned null".to_string());
        }
        let jstr = jni::objects::JString::from(obj);
        let s: String = env.get_string(&jstr).map_err(|e| e.to_string())?.into();
        Ok(s)
    }

    pub fn push_payload(payload: &str) -> Result<bool, String> {
        let vm = get_java_vm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach: {e}"))?;
        let cls = find_app_class(&mut env, "dev.frazse.terra.MainActivity")
            .map_err(|e| format!("find_class: {e}"))?;
        let j_payload = env
            .new_string(payload)
            .map_err(|e| format!("new_string: {e}"))?;
        let res = env
            .call_static_method(
                &cls,
                "pushDualScreenPayload",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&j_payload)],
            )
            .map_err(|e| {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let msg = format!("call pushDualScreenPayload: {e}");
                alog(&msg);
                msg
            })?;
        res.z().map_err(|e| format!("result pushDualScreenPayload: {e}"))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_dualscreen_status() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let raw = jni_dual::get_status_json()?;
        serde_json::from_str(&raw).map_err(|e| format!("invalid dualscreen status json: {e}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        Ok(serde_json::json!({ "supported": false }))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_dualscreen_configure(
    enabled: Option<bool>,
    mode: Option<String>,
    keep_nameplates: Option<bool>,
    in_ride: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let current = terra_dualscreen_status().unwrap_or_else(|_| serde_json::json!({}));
        let cur_enabled = current
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let cur_mode = current
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("cockpit");
        let cur_np = current
            .get("keepNameplates")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let cur_ride = current
            .get("inRide")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let next_mode = match mode.as_deref() {
            Some("mirror") => "mirror",
            Some("cockpit") => "cockpit",
            _ => cur_mode,
        };

        let raw = jni_dual::configure(
            enabled.unwrap_or(cur_enabled),
            next_mode,
            keep_nameplates.unwrap_or(cur_np),
            in_ride.unwrap_or(cur_ride),
        )?;
        serde_json::from_str(&raw).map_err(|e| format!("invalid dualscreen config json: {e}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (enabled, mode, keep_nameplates, in_ride);
        Ok(serde_json::json!({ "supported": false }))
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub fn terra_dualscreen_push(payload: String) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let ok = jni_dual::push_payload(&payload)?;
        Ok(serde_json::json!({ "ok": ok }))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = payload;
        Ok(serde_json::json!({ "ok": false }))
    }
}
