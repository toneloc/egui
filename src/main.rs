mod types;
mod price_feeds;
mod stable;

use eframe::{egui, App, Frame};
use egui::{epaint,TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use ldk_node::{
    bitcoin::{address, secp256k1::PublicKey, Address, Network}, lightning::{
        ln::{msgs::SocketAddress, types::ChannelId},
        offers::offer::Offer,
    }, Builder, ChannelDetails, Node
};
use hex;
use qrcode::{Color, QrCode};
use stable::get_latest_price;
use std::time::{Duration, Instant};
use types::{Bitcoin, StableChannel, USD};
use ldk_node::Event;

use crate::stable::{check_stability,close_channels_to_address};

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
    
    
    let mut dir = dirs::home_dir().unwrap();
    dir.push("sc-data");
    dir.push(alias);
    builder.set_storage_dir_path(dir.to_string_lossy().to_string());
    
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

        let stable_channel = StableChannel {
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
            latest_price: get_latest_price(),
            prices: "".to_string(),
        };

        println!(
            "Stable Channel created: {:?}",
            stable_channel.channel_id.to_string()
        );

        // Determine the initial app state based on channel_id
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
        }
    }
        // Check if we already have a channel open

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Stable Channels v0.2").size(28.0).strong(),
                );
                ui.add_space(150.0);

                let create_channel_button = egui::Button::new(
                    egui::RichText::new("Create Stable Channel")
                        .color(egui::Color32::BLACK)
                        .size(18.0),
                        )
                        .min_size(egui::vec2(200.0, 55.0))
                        .fill(egui::Color32::from_gray(220))
                        .rounding(8.0); // Subtle rounded corners

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
            ui.add_space(15.0);

            ui.vertical_centered(|ui| {
                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add_space(15.0);


                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                ui.add_space(15.0);

                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Copy Invoice")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(150.0, 45.0)) // Smaller button size
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0),
                ).clicked() {
                    ctx.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }
                
                ui.add_space(10.0); 
                
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Back")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(150.0, 45.0)) 
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
            egui::Frame::none()
                .inner_margin(epaint::Margin::symmetric(20.0, 0.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        let balances = self.user.list_balances();
                        let lightning_balance_btc = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                        let lightning_balance_usd =
                            USD::from_bitcoin(lightning_balance_btc, self.stable_channel.latest_price);
    
                        ui.add_space(30.0);
    
                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.add(egui::Label::new(
                                egui::RichText::new(lightning_balance_usd.to_string())
                                    .size(36.0)
                                    .strong(),
                            ));
        
                            ui.label(format!("{}", lightning_balance_btc.to_string()));
    
                            ui.add_space(20.0);
                        });

                        ui.add_space(20.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price");
                            ui.label(format!("${:.2}", self.stable_channel.latest_price));
                            ui.add_space(20.0);
                        });

                        ui.add_space(50.0);
                        
                        ui.label("Withdrawal address (minus transaction fees):");
                        
                        ui.add_space(10.0);
                        ui.text_edit_singleline(&mut self.close_channel_address);
                        ui.add_space(10.0);
                        if ui.add(
                            egui::Button::new(
                                egui::RichText::new("Close Channel")
                                    .color(egui::Color32::BLACK)
                                    .size(16.0), 
                            )
                            .min_size(egui::vec2(150.0, 45.0)) 
                            .fill(egui::Color32::from_gray(220))
                            .rounding(6.0),)
                            .clicked() {
                                self.status_message = format!(
                                    "Your Stable Channel is closing and withdrawal transaction to {} is processing.",
                                    self.close_channel_address
                                );

                            close_channels_to_address(
                                &self.user,
                                self.close_channel_address.clone()
                            );
                        }
    
                        ui.add_space(20.0);

                        if !self.status_message.is_empty() {
                            ui.label(self.status_message.clone());
                        }
                    });
                });
        });

    }

    fn show_closing_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.heading(
                    egui::RichText::new(format!("Your withdrawal transaction to {} is processing", self.close_channel_address)).size(28.0).strong(),
                );
                ui.add_space(20.0);
            });
        });
    }

    fn poll_for_events(&mut self) {
        while let Some(event) = self.user.next_event() {
            match event {
                Event::ChannelReady { .. } => {
                    self.state = AppState::MainScreen;
                }
                
                // update UI balances
                Event::PaymentReceived { .. } => {
                    self.state = AppState::MainScreen;
                    println!("payment received");
                }

                // update UI balances
                Event::ChannelClosed { .. } => {
                    self.state = AppState::ClosingScreen;
                    println!("channel closed");


                }
                // Wildcard
                _ => {
                
                }
            }
            self.user.event_handled();
        }
    }

}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        let now = Instant::now();
        
        if now.duration_since(self.last_stability_check) >= Duration::from_secs(30) {
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
    std::panic::set_hook(Box::new(|info| {
        eprintln!("Application panicked: {}", info);
    }));
    
    println!("Starting the app...");
    let native_options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    );
    println!("App has exited.");
}
