use eframe::{egui, App, Frame};
use ldk_node::{bitcoin::{secp256k1::PublicKey, Network}, lightning::ln::msgs::SocketAddress, Builder};

struct UserData {
    is_onboarding: bool,
    has_paid_initial_invoice: bool,
    waiting_for_onboarding: bool,
    public_key: u64,
}

impl Default for UserData {
    fn default() -> Self {
        Self {
            is_onboarding: true,
            has_paid_initial_invoice: false,
            waiting_for_onboarding: false,
            public_key: 0x123,
        }
    }
}

struct MyApp {
    user_data: UserData,
    invoice_result: String,
    user: ldk_node::Node,
}

/// LDK set-up and initialization
fn make_node(alias: &str, port: u16, lsp_pubkey: Option<PublicKey>) -> ldk_node::Node {
    let mut builder = Builder::new();

    // If we pass in an LSP pubkey then set your liquidity source
    if let Some(lsp_pubkey) = lsp_pubkey {
        println!("{}", lsp_pubkey.to_string());
        let address = "127.0.0.1:9376".parse().unwrap();
        builder.set_liquidity_source_lsps2(
            address,
            lsp_pubkey,
            Some("00000000000000000000000000000000".to_owned()),
        );
    }

    builder.set_network(Network::Signet);

    // If this doesn't work, try the other one
    builder.set_chain_source_esplora("https://mutinynet.com/api/".to_string(), None);
    // builder.set_esplora_server("https://mutinynet.ltbl.io/api".to_string());

    // Don't need gossip right now. Also interferes with Bolt12 implementation.
    // builder.set_gossip_source_rgs("https://mutinynet.ltbl.io/snapshot".to_string());
    builder.set_storage_dir_path(("./data/".to_owned() + alias).to_string());
    let _ = builder.set_listening_addresses(vec![format!("127.0.0.1:{}", port).parse().unwrap()]);
    let _ = builder.set_node_alias("some_alias".to_string()); // needed to open announced channel since LDK 0.4.0

    let node = builder.build().unwrap();
    node.start().unwrap();
    let public_key: PublicKey = node.node_id();

    let listening_addresses: Vec<SocketAddress> = node.listening_addresses().unwrap();

    if let Some(first_address) = listening_addresses.first() {
        println!("");
        println!("Actor Role: {}", alias);
        println!("Public Key: {}", public_key);
        println!("Internet Address: {}", first_address);
        println!("");
    } else {
        println!("No listening addresses found.");
    }

    return node;
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let user = make_node("user", 9735, None);
        Self {
            user_data: UserData::default(),
            invoice_result: String::new(),
            user,
        }
    }

    fn get_jit_invoice(&mut self) {
        match self.user.bolt11_payment().receive_via_jit_channel(
            50000000,
            "Stable Channel",
            3600,
            Some(10000000),
        ) {
            Ok(invoice) => {
                println!("Invoice: {:?}", invoice);
                self.invoice_result = format!("Invoice: {:?}", invoice);
            }
            Err(e) => {
                println!("Error: {:?}", e);
                self.invoice_result = format!("Error: {:?}", e);
            }
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Stable Channels ⚖️💵⚡");

                if self.user_data.is_onboarding {
                    if !self.user_data.waiting_for_onboarding {
                        if ui.button("create a $100 stable channel").clicked() {
                            self.get_jit_invoice();
                            self.user_data.waiting_for_onboarding = true;
                        }
                    } else {
                        ui.add(egui::TextEdit::multiline(&mut String::new()).desired_rows(10));
                        ui.label(&self.invoice_result);
                        if ui.button("advance").clicked() {
                            self.user_data.waiting_for_onboarding = false;
                            self.user_data.is_onboarding = false;
                        }
                    }
                } else {
                    ui.heading("$100.000");
                    ui.label(".00234 bitcoin");
                }
            });
        });
    }
}

fn main() {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "My App",
        native_options,
        Box::new(|cc| Box::new(MyApp::new(cc))),
    );
}
