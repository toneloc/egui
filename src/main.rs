mod config;
mod stable;
mod types;
mod price_feeds;

use eframe::{egui, App, Frame};
use egui::{epaint::{self, Margin}, TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use ldk_node::{
    bitcoin::{address, secp256k1::PublicKey, Address, Network},
    lightning::{ln::{msgs::SocketAddress, types::ChannelId}, offers::offer::Offer},
    Builder, ChannelDetails, Node, Event,
};
use lightning::routing::gossip::NodeId;
use qrcode::{Color, QrCode};
use std::{str::FromStr, time::{Duration, Instant}};
use dirs_next as dirs;

use crate::config::Config;
use crate::stable::{check_stability, close_channels_to_address, get_latest_price};
use crate::types::{Bitcoin, StableChannel, USD};

enum AppState {
    OnboardingScreen,
    WaitingForPayment,
    MainScreen,
    ClosingScreen
}

struct MyApp {
    state: AppState,
    last_stability_check: Instant,
    invoice_result: String,
    user: Node,
    qr_texture: Option<TextureHandle>,
    channel_list: Vec<ChannelDetails>,
    stable_channel: StableChannel,
    close_channel_address: String,
    status_message: String,
    config: Config,  // store our loaded config
}

fn make_node(config: &Config, lsp_pubkey: Option<PublicKey>) -> Node {
    let mut builder = Builder::new();

    // Use values from config rather than hardcoding
    if let Some(lsp_pubkey) = lsp_pubkey {
        let address = config.lsp.address.parse().unwrap();
        builder.set_liquidity_source_lsps2(
            address,
            lsp_pubkey,
            Some(config.lsp.auth.clone()),
        );
    }

    let network = match config.node.network.to_lowercase().as_str() {
        "signet" => Network::Signet,
        "testnet" => Network::Testnet,
        "bitcoin" => Network::Bitcoin,
        // fallback
        _ => Network::Signet,
    };

    builder.set_network(network);
    builder.set_chain_source_esplora(config.node.chain_source_url.clone(), None);

    let mut dir = dirs::home_dir().unwrap();
    dir.push(&config.node.data_dir);
    dir.push(&config.node.alias);
    builder.set_storage_dir_path(dir.to_string_lossy().to_string());

    builder
        .set_listening_addresses(vec![format!("127.0.0.1:{}", config.node.port)
        .parse()
        .unwrap()])
        .unwrap();

    builder.set_node_alias(config.node.alias.clone());

    let node = builder.build().unwrap();
    node.start().unwrap();

    if let Some(first_address) = node.listening_addresses().unwrap().first() {
        println!("\nActor Role: {}", config.node.alias);
        println!("Public Key: {}", node.node_id());
        println!("Internet Address: {}\n", first_address);
    } else {
        println!("No listening addresses found.");
    }
    node
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        let lsp_pubkey_bytes = hex::decode(&config.lsp.pubkey).unwrap();
        let lsp_pubkey = PublicKey::from_slice(&lsp_pubkey_bytes).unwrap();
        println!("{}", lsp_pubkey);

        let user = make_node(&config, Some(lsp_pubkey));
        let channel_id = if !user.list_channels().is_empty() {
            user.list_channels()[0].channel_id
        } else {
            ChannelId::from_bytes([0; 32])
        };

        let stable_channel = StableChannel {
            channel_id,
            is_stable_receiver: true,
            counterparty: lsp_pubkey,
            expected_usd: USD::from_f64(config.stable_channel_defaults.expected_usd),
            expected_btc: Bitcoin::from_btc(0.0),
            stable_receiver_btc: Bitcoin::from_btc(0.0),
            stable_provider_btc: Bitcoin::from_btc(0.0),
            stable_receiver_usd: USD::from_f64(0.0),
            stable_provider_usd: USD::from_f64(0.0),
            risk_level: 0,
            timestamp: 0,
            formatted_datetime: "2021-06-01 12:00:00".to_string(),
            payment_made: false,
            sc_dir: config.stable_channel_defaults.sc_dir.clone(),
            latest_price: get_latest_price(),
            prices: "".to_string(),
        };
        println!("Stable Channel created: {:?}", stable_channel.channel_id.to_string());

        // Initialize the app state
        let state = if channel_id != ChannelId::from_bytes([0; 32]) {
            AppState::MainScreen
        } else {
            AppState::OnboardingScreen
        };

        Self {
            state,
            last_stability_check: Instant::now() - Duration::from_secs(60),
            invoice_result: String::new(),
            user,
            qr_texture: None,
            channel_list: Vec::new(),
            stable_channel,
            close_channel_address: String::new(),
            status_message: String::new(),
            config,
        }
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Stable Channels v0.2")
                        .size(28.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(50.0);
    
                // Step 1
                ui.heading(
                    egui::RichText::new("Step 1: Get a Lightning invoice ⚡")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(r#"Press the "Stabilize" button below."#)
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 2
                ui.heading(
                    egui::RichText::new("Step 2: Send yourself bitcoin 💸")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("From a Lightning app or an exchange.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 3
                ui.heading(
                    egui::RichText::new("Step 3: Stable channel created 🔧")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Self-custody. Your keys, your coins.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(50.0);
    
                // Create channel button
                let subtle_orange = egui::Color32::from_rgba_premultiplied(247, 147, 26, 200); 
                let create_channel_button = egui::Button::new(
                    egui::RichText::new("Stabilize")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                )
                .min_size(egui::vec2(200.0, 55.0))
                .fill(subtle_orange)
                .rounding(8.0);
    
                if ui.add(create_channel_button).clicked() {
                    self.get_jit_invoice(ctx);
                    self.state = AppState::WaitingForPayment;
                }
            });
        });
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);

            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Send yourself bitcoin to stabilize.")
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(10.0);

                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add_space(10.0);


                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                ui.add_space(10.0);

                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Copy Invoice")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0),
                ).clicked() {
                    ctx.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }
                
                ui.add_space(5.0); 
                
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Back")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0), 
                ).clicked() {
                    self.state = AppState::OnboardingScreen;
                }
                
                ui.add_space(10.0); 
            });
        });
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let result = self.user.bolt11_payment().receive_via_jit_channel(
            18_780_000,
            "Stable Channel",
            3600,
            Some(10_000_000),
        );
    
        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                let code = QrCode::new(&self.invoice_result).unwrap();
                let bits = code.to_colors();
                let width = code.width();
                let scale_factor = 4;
                let mut imgbuf =
                    GrayImage::new((width * scale_factor) as u32, (width * scale_factor) as u32);
    
                for y in 0..width {
                    for x in 0..width {
                        let color = if bits[y * width + x] == Color::Dark {
                            0
                        } else {
                            255
                        };
                        for dy in 0..scale_factor {
                            for dx in 0..scale_factor {
                                imgbuf.put_pixel(
                                    (x * scale_factor + dx) as u32,
                                    (y * scale_factor + dy) as u32,
                                    Luma([color]),
                                );
                            }
                        }
                    }
                }
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for pixel in imgbuf.pixels() {
                    let lum = pixel[0];
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(255);
                }
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.qr_texture =
                    Some(ctx.load_texture("qr_code", color_image, TextureOptions::LINEAR));
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
            }
        }
    }
    
    fn show_main_screen(&mut self, ctx: &egui::Context) {    
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::none()
                .inner_margin(epaint::Margin::symmetric(20.0, 0.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        let balances = self.user.list_balances();

                        let lightning_balance_btc = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                        let lightning_balance_usd = USD::from_bitcoin(lightning_balance_btc,self.stable_channel.latest_price);
                        ui.add_space(30.0);
    
                        ui.group(|ui| {
                            ui.add_space(20.0);
                        
                            ui.heading("Your Stable Balance");

                            ui.add(egui::Label::new(
                                egui::RichText::new(lightning_balance_usd.to_string())
                                    .size(36.0)
                                    .strong(),
                            ));
        
                            ui.label(format!("Agreed Peg USD: {}", self.stable_channel.expected_usd));

                            ui.label(format!("Bitcoin: {}", lightning_balance_btc.to_string()));
    
                            ui.add_space(20.0);
                        });

                        ui.add_space(20.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price ");
                            ui.label(format!("${:.2}", self.stable_channel.latest_price));
                            ui.add_space(20.0);
                        
                        let last_updated = self.last_stability_check.elapsed().as_secs();
                        ui.add_space(5.0);
                        ui.label(
                            egui::RichText::new(format!("Last updated: {}s ago", last_updated))
                                .size(12.0)
                                .color(egui::Color32::GRAY),
                        );
                        });


                        ui.add_space(20.0);
                        
                        ui.collapsing("Close Channel", |ui| {
                            ui.label("Withdrawal address (minus transaction fees):");
                        
                            ui.add_space(10.0);
                            ui.text_edit_singleline(&mut self.close_channel_address);
                            ui.add_space(10.0);
                        
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new("Close Channel")
                                        .color(egui::Color32::WHITE)
                                        .size(12.0),
                                )
                                .rounding(6.0),
                            )
                            .clicked()
                            {
                                close_channels_to_address(&self.user, self.close_channel_address.clone());
                            }
                        
                            ui.add_space(20.0);
                        
                            if !self.status_message.is_empty() {
                                ui.label(self.status_message.clone());
                            }
                        });
                        
                    });
                });
        });

    }

    fn show_closing_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.heading(
                    egui::RichText::new(format!("Withdrawal processing")).size(28.0).strong(),
                );    
            });
    
            ui.add_space(20.0);
            ui.horizontal_centered(|ui| {
                ui.heading(                    
                    egui::RichText::new(format!("{}",self.close_channel_address)).size(28.0).strong(), 
                );
            });
        });
    }

    fn poll_for_events(&mut self) {
        while let Some(event) = self.user.next_event() {
            match event {
                Event::ChannelReady { .. } => {
                    self.state = AppState::MainScreen;
                }
                
                Event::PaymentReceived { .. } => {
                    self.state = AppState::MainScreen;
                    println!("payment received");
                }

                Event::ChannelClosed { .. } => {
                    self.state = AppState::ClosingScreen;
                    println!("channel closed");


                }
                _ => {
                
                }
            }
            self.user.event_handled();
        }
    }

    pub fn connect_to_lsp_and_entry_node(&mut self) {
        let _connected_to_lsp = self.user.connect(
            PublicKey::from_str("0367631f3a8ca46bccf6d8eae8b728963337f8a6825199386c9a48987ea82b54cd")
                .unwrap(),
            SocketAddress::from_str("127.0.0.1:9737").unwrap(),
            true,
        );
    
        println!("Connection result: {:?}", _connected_to_lsp.unwrap());
    
        let _connected_to_exchange = self.user.connect(
            PublicKey::from_str("03e9d73c317a6113a30e85d7dafcebaa509c1744e0528d392ae975d2e4177d11dc")
                .unwrap(),
            SocketAddress::from_str("127.0.0.1:9735").unwrap(),
            true,
        );
    
        println!("Connection result: {:?}", _connected_to_exchange.unwrap());
    
        let node_info = self.user
            .network_graph()
            .node(&NodeId::from_pubkey(
                &PublicKey::from_str("03ebdb4d14e3101c1d63e3d5555db2d15bc50d32bc30919b7dfd3d35609b978ff4")
                    .unwrap(),
            ));
    
        println!("Node information: {:?}", node_info);
    
        let node_info = self.user
            .network_graph()
            .node(&NodeId::from_pubkey(
                &PublicKey::from_str("0367631f3a8ca46bccf6d8eae8b728963337f8a6825199386c9a48987ea82b54cd")
                    .unwrap(),
            ));
    
        println!("Node information: {:?}", node_info);
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        let now = Instant::now();
        
        if now.duration_since(self.last_stability_check) >= Duration::from_secs(30) {
            // self.connect_to_lsp_and_entry_node();
            check_stability(&self.user, &mut self.stable_channel);
            self.last_stability_check = now;
        }

        match self.state {
            AppState::OnboardingScreen => self.show_onboarding_screen(ctx),
            AppState::WaitingForPayment => self.show_waiting_for_payment_screen(ctx),
            AppState::MainScreen => self.show_main_screen(ctx),
            AppState::ClosingScreen => self.show_closing_screen(ctx),
        }

        self.poll_for_events();
        
    }
}
fn main() {
    let config = config::Config::from_file("src/config.toml");

    let native_options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc, config)))),
    );
    println!("App has exited.");
}
