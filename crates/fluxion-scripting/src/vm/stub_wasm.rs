// ============================================================
// fluxion-scripting — JsVm stub (WASM: no QuickJS / rquickjs-sys)
// ============================================================

/// Placeholder VM: scripting is disabled on `wasm32` until a web JS bridge exists.
pub struct JsVm;

impl JsVm {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn eval(&self, _source: &str, _name: &str) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn load_script(&self, _path: &str) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn eval_string_result(&self, _source: &str, name: &str) -> anyhow::Result<String> {
        match name {
            "<script-target-names>" => Ok("[]".to_string()),
            "<collect-script-transforms>" => Ok("{}".to_string()),
            _ => Ok("{}".to_string()),
        }
    }

    pub fn update(&self, _dt: f32) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn fixed_update(&self, _fixed_dt: f32) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn set_global<T>(&self, _name: &str, _value: T) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn get_global<T>(&self, name: &str) -> anyhow::Result<T> {
        anyhow::bail!("JsVm::get_global unavailable on wasm32 stub (name={name})")
    }
}
