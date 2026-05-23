use std::fs;
use std::path::Path;
use ts_rs::TS;
use pi_core::events::{AgentAction, AgentEvent};
use pi_core::message::AgentMessage;
use pi_core::agent::AgentState;
use pi_core::tool::ToolDefinition;
use pi_core::llm::Model;

fn export<T: TS + 'static>(dir: &Path, config: &ts_rs::Config) {
    let output = T::export_to_string(config).unwrap_or_else(|e| panic!("export failed: {e}"));
    let name = T::name(config);
    let path = dir.join(format!("{name}.ts"));
    fs::write(&path, &output).unwrap_or_else(|e| panic!("write failed: {e}"));
    eprintln!("  wrote {}", path.display());
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        format!("{}/../../web/src/types", std::env::var("CARGO_MANIFEST_DIR").unwrap())
    });
    let dir = Path::new(&out_dir);
    fs::create_dir_all(dir).ok();
    let config = ts_rs::Config::default();

    eprintln!("Generating TypeScript types in {out_dir}/");
    export::<AgentEvent>(dir, &config);
    export::<AgentAction>(dir, &config);
    export::<AgentMessage>(dir, &config);
    export::<AgentState>(dir, &config);
    export::<ToolDefinition>(dir, &config);
    export::<Model>(dir, &config);
    eprintln!("Done.");
}
