// The launcher calls the Grid Editor makes, answered from the helper's context: it
// was written as a helper of its own, asking the launcher by name over the pipe, and
// the context makes the same calls.
use serde_json::{Value, json};
use swofty_addon::launcher::Cx;

pub struct Host<'a> {
    cx: &'a Cx<'a>,
}

impl<'a> Host<'a> {
    pub fn new(cx: &'a Cx<'a>) -> Self {
        Self { cx }
    }

    /// The calls this launcher answers, for the editor's checks before it makes one.
    pub fn calls(cx: &Cx) -> Vec<&'static str> {
        let mut out = vec!["profile_heads"];
        if cx.addons().is_some() {
            out.extend(["addons", "install_addon", "move_addon"]);
        }
        out
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let text = |e: swofty_addon::Error| e.to_string();
        let game = || {
            self.cx
                .game
                .ok_or_else(|| "the launcher hasn't found F1 25".to_owned())
        };
        let path = |p: std::path::PathBuf| json!(p.to_string_lossy());
        match method {
            "game_build_key" => Ok(json!(game()?.build_key())),
            "game_dir" => Ok(game()?.dir().map_or(Value::Null, path)),
            "game_original" => {
                let rel = params["rel"].as_str().ok_or("game_original needs rel")?;
                game()?.original(rel).map(path).map_err(text)
            }
            "game_original_lang" => {
                let code = params["code"].as_str().ok_or("game_original_lang needs code")?;
                game()?.original_lang(code).map(path).map_err(text)
            }
            "profile_heads" => {
                let heads = game()?.profile_heads(&self.cx.profile.name).map_err(text)?;
                serde_json::to_value(heads).map_err(|e| e.to_string())
            }
            "addons" => {
                let store = self.cx.addons().ok_or("this launcher doesn't list addons")?;
                serde_json::to_value(store.list().map_err(text)?).map_err(|e| e.to_string())
            }
            "install_addon" => {
                let store = self.cx.addons().ok_or("this launcher can't install addons")?;
                let at = params["path"].as_str().ok_or("install_addon needs path")?;
                serde_json::to_value(store.install(std::path::Path::new(at)).map_err(text)?)
                    .map_err(|e| e.to_string())
            }
            other => Err(format!(
                "the Grid Editor asked the launcher for {other}, which it doesn't answer here"
            )),
        }
    }
}
