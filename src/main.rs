mod types;
mod price_feeds;
mod stable;

use eframe::{egui, App, Error, Frame};
use egui::{Color32, Frame as EguiFrame, Margin, Stroke, TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use ldk_node::{
    bitcoin::{address, secp256k1::PublicKey, Address, Network}, lightning::{
        ln::{msgs::SocketAddress, types::ChannelId},
        offers::offer::Offer,
    }, Builder, ChannelDetails, Node
};
use hex;
use qrcode::{Color, QrCode};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, time::{Duration, Instant}};
use types::{Bitcoin, StableChannel, USD};
use ldk_node::Event;

use crate::stable::{check_stability,list_channels, close_channels_to_address, connect_to_lsp_and_entry_node};

enum AppState {
    HomeScreen,
    WaitingForPayment,
    MainScreen,
}

struct MyApp {
    state: AppState,
    last_stability_check: Instant,
    invoice_result: String,
    user: Node,
    qr_texture: Option<TextureHandle>,
    channel_list: Vec<ChannelDetails>,
    channel_list_string: String,
    stable_channel: StableChannel,
    showing_channels: bool,
    close_channel_address: String,
    network: Network,
    frame: EguiFrame,
}

fn make_node(alias: &str, port: u16, lsp_pubkey: Option<PublicKey>) -> Node {
    let mut builder = Builder::new();
    if let Some(lsp_pubkey) = lsp_pubkey {
        let address = "127.0.0.1:9737".parse().unwrap();
        builder.set_liquidity_source_lsps2(
            address,
            lsp_pubkey,
            Some("00000000000000000000000000000000".to_owned()),
        );
    }
    builder.set_network(Network::Signet);
    builder.set_chain_source_esplora("https://mutinynet.com/api/".to_string(), None);
    builder.set_storage_dir_path(format!("./data/{alias}"));
    let _ = builder.set_listening_addresses(vec![format!("127.0.0.1:{port}").parse().unwrap()]);
    let _ = builder.set_node_alias("some_alias".to_string());

    let node = builder.build().unwrap();
    node.start().unwrap();

    let listening_addresses: Vec<SocketAddress> = node.listening_addresses().unwrap();

    if let Some(first_address) = listening_addresses.first() {
        println!("");
        println!("Actor Role: {}", alias);
        println!("Public Key: {}", node.node_id());
        println!("Internet Address: {}", first_address);
        println!("");
    } else {
        println!("No listening addresses found.");
    }

    node
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let bytes = hex::decode(
            "03299e2ebafc734bf2759867c34ea4533e1c275ff1399bc6c4be099a18d625d7e3",
        )
        .unwrap();
        let lsp_pubkey = PublicKey::from_slice(&bytes).ok().unwrap();
        let user = make_node("user", 9736, Some(lsp_pubkey));

        let channel_id = if !user.list_channels().is_empty() {
            user.list_channels()[0].channel_id
        } else {
            ChannelId::from_bytes([0; 32]) // set to zero to start
        };

        // you should store stable amt in a comment in a small payment
        // check it is signed by stable provider!!!

        let mut stable_channel = StableChannel {
            channel_id,
            is_stable_receiver: true,
            counterparty: lsp_pubkey,
            expected_usd: USD::from_f64(28.0),
            expected_btc: Bitcoin::from_btc(0.0),
            stable_receiver_btc: Bitcoin::from_btc(0.0),
            stable_provider_btc: Bitcoin::from_btc(0.0),
            stable_receiver_usd: USD::from_f64(0.0),
            stable_provider_usd: USD::from_f64(0.0),
            risk_level: 0,
            timestamp: 0,
            formatted_datetime: "2021-06-01 12:00:00".to_string(),
            payment_made: false,
            sc_dir: "/path/to/sc_dir".to_string(),
            latest_price: 0.0,
            prices: "".to_string(),
        };

        println!(
            "Stable Channel created: {:?}",
            stable_channel.channel_id.to_string()
        );

        let frame = EguiFrame::none()
            .inner_margin(egui::Margin::same(10.0))
            .outer_margin(Margin::same(16.0))
            .stroke(Stroke::new(3.0, Color32::BLACK))
            .fill(egui::Color32::WHITE)
            .rounding(10.0)
            .shadow(egui::Shadow {
                offset: egui::Vec2::new(0.0, 0.0),
                blur: 30.0,
                spread: 0.0,
                color: Color32::from_rgba_unmultiplied(255, 255, 255, 80),
        });

        // Determine the initial app state based on channel_id
        let state = if channel_id != ChannelId::from_bytes([0; 32]) {
            AppState::MainScreen
        } else {
            AppState::HomeScreen
        };

        Self {
            state,
            last_stability_check: Instant::now() - Duration::from_secs(60),
            invoice_result: String::new(),
            user,
            qr_texture: None,
            channel_list: Vec::new(),
            channel_list_string: String::new(),
            stable_channel,
            showing_channels: false,
            close_channel_address: String::new(),
            network: Network::Signet,
            frame,
        }
    }

    fn check_state(&mut self) {
        // Check if we already have a channel open
        
    }

    fn show_home_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Stable Channels ⚖️💵⚡");
                ui.add_space(80.0);

                let create_channel_button = egui::Button::new("Create a $100 stable channel")
                    .min_size(egui::vec2(180.0, 50.0));

                if ui.add(create_channel_button).clicked() {
                    // connect_to_lsp_and_entry_node(&self.user);
                    self.get_jit_invoice(ctx);
                    self.state = AppState::WaitingForPayment;
                }
            });
        });
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                if ui.button("Copy Invoice").clicked() {
                    ctx.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }

                if !self.channel_list.is_empty() {
                    self.state = AppState::MainScreen;
                }

                if ui.button("Back").clicked() {
                    self.state = AppState::HomeScreen;
                }
            });
        });
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        // let _connected = self.user.connect(
        //     PublicKey::from_str("02e897f0ce1bf88afe1f8e2be0045294ec87b00eebd689e42ba7290cfa2922dbe7")
        //         .unwrap(),
        //     SocketAddress::from_str("127.0.0.1:9735").unwrap(),
        //     true,
        // );
    
        let result = self.user.bolt11_payment().receive_via_jit_channel(
            30_000_000,
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
            ui.vertical_centered(|ui| {
                let balances = self.user.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);

                ui.label(format!("On-Chain Balance: {}", onchain_balance));
                ui.label(format!("Lightning Balance: {}", lightning_balance));

                if ui.button("List Channels").clicked() {
                    let (_channels, info) = list_channels(&self.user);
                    ui.label(info);
                }

                ui.text_edit_singleline(&mut self.close_channel_address);
                if ui.button("Close channel to address").clicked() {
                    close_channels_to_address(&self.user, self.close_channel_address.clone());
                }

                if ui.button("Back").clicked() {
                    self.state = AppState::HomeScreen;
                }
            });
        });

}

    fn poll_for_events(&mut self) {
        while let Some(event) = self.user.next_event() {
            match event {
                Event::ChannelReady { .. } => {
                    self.state = AppState::MainScreen;
                }
                _ => {}
            }
            self.user.event_handled();
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {

        self.check_state();

        match self.state {
            AppState::HomeScreen => self.show_home_screen(ctx),
            AppState::WaitingForPayment => self.show_waiting_for_payment_screen(ctx),
            AppState::MainScreen => self.show_main_screen(ctx),
        }

        self.poll_for_events();
    }
}
  
fn main() {
    let native_options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    );
}
