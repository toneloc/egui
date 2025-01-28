use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct LspConfig {
    pub pubkey: String,
    pub address: String,
    pub auth: String,
}

#[derive(Deserialize, Debug)]
pub struct NodeConfig {
    pub network: String,
    pub chain_source_url: String,
    pub data_dir: String,
    pub alias: String,
    pub port: u16,
}

#[derive(Deserialize, Debug)]
pub struct StableChannelConfig {
    pub expected_usd: f64,
    pub sc_dir: String,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub lsp: LspConfig,
    pub node: NodeConfig,
    pub stable_channel_defaults: StableChannelConfig,
}

impl Config {
    pub fn from_file(path: &str) -> Self {
        let content = std::fs::read_to_string(path)
            .expect("Unable to read configuration file.");
        toml::from_str(&content)
            .expect("Invalid format in configuration file.")
    }
}
