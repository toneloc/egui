use std::{any::Any, str::FromStr};

mod types;
mod price_feeds;

use eframe::{egui, App, Frame};
use egui::{TextureHandle, TextureOptions};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Address, Network},
    lightning::{
        ln::{msgs::SocketAddress, types::ChannelId},
        offers::offer::Offer,
    },
    Builder, Node, ChannelDetails,
};
use hex;
use image::{GrayImage, Luma};
use qrcode::{Color, QrCode};
use ureq::Agent;

use std::time::{Instant, Duration};

use types::{Bitcoin, StableChannel, USD};
use price_feeds::{calculate_median_price, fetch_prices, set_price_feeds};


#[derive(Clone)]
struct UserData {
    is_onboarding: bool,
    has_paid_initial_invoice: bool,
    waiting_for_invoice_payment: bool,
    public_key: u64,
}

impl Default for UserData {
    fn default() -> Self {
        Self {
            is_onboarding: true,
            has_paid_initial_invoice: false,
            waiting_for_invoice_payment: false,
            public_key: 0x123,
        }
    }
}

struct MyApp {
    last_stability_check: Instant,
    user_data: UserData,
    invoice_result: String,
    user: Node,
    qr_texture: Option<TextureHandle>,
    channel_list: Vec<ChannelDetails>,
    channel_list_string: String,
    dot_counter: usize,
    stable_channel: StableChannel,
    showing_channels: bool,
    close_channel_address: String,
    network: Network,
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
    node
}

impl MyApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let bytes = hex::decode(
            "03c5a9b32688c82cc1efa7c205390ef10444d8d6a412af91aa429f7bf34bb19c11",
        )
        .unwrap();
        let lsp_pubkey = PublicKey::from_slice(&bytes).ok().unwrap();
        let user = make_node("user", 9736, Some(lsp_pubkey));

        let channel_id_bytes: [u8; 32] = [0; 32];

        let mut stable_channel = StableChannel {
            channel_id: ChannelId::from_bytes(channel_id_bytes),
            is_stable_receiver: true,
            counterparty: lsp_pubkey,
            expected_usd: USD::from_f64(0.0),
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

        Self {
            last_stability_check: Instant::now(),
            user_data: UserData::default(),
            invoice_result: String::new(),
            user,
            qr_texture: None,
            channel_list: Vec::new(),
            channel_list_string: String::new(),
            dot_counter: 0,
            stable_channel,
            showing_channels: false,
            close_channel_address: String::new(),
            network: Network::Signet,
        }
    }

    /// Core stability logic
    fn check_stability(node: &Node, mut sc: StableChannel) -> StableChannel {
        sc.latest_price = fetch_prices(&Agent::new(), &set_price_feeds())
        .and_then(|prices| calculate_median_price(prices))
        .unwrap_or(0.0);

        if let Some(channel) = node
            .list_channels()
            .iter()
            .find(|c| c.channel_id == sc.channel_id)
        {
            sc = Self::update_balances(sc, Some(channel.clone()));
        }

        let mut dollars_from_par: USD = sc.stable_receiver_usd - sc.expected_usd;
        let mut percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();

        println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
        println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);
        println!("{:<25} {:>5}", "Percent from par:", format!("{:.2}%\n", percent_from_par));

        println!("{:<25} {:>15}", "User BTC:", sc.stable_receiver_btc);
        println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);

        enum Action {
            Wait,
            Pay,
            DoNothing,
            HighRisk,
        }

        let action = if percent_from_par < 0.1 {
            Action::DoNothing
        } else {
            let is_receiver_below_expected: bool = sc.stable_receiver_usd < sc.expected_usd;

            match (sc.is_stable_receiver, is_receiver_below_expected, sc.risk_level > 100) {
                (_, _, true) => Action::HighRisk, // High risk scenario
                (true, true, false) => Action::Wait,   // We are User and below peg, wait for payment
                (true, false, false) => Action::Pay,   // We are User and above peg, need to pay
                (false, true, false) => Action::Pay,   // We are LSP and below peg, need to pay
                (false, false, false) => Action::Wait, // We are LSP and above peg, wait for payment
            }
        };

        match action {
            Action::DoNothing => println!("\nDifference from par less than 0.1%. Doing nothing."),
            Action::Wait => {
                println!("\nWaiting 10 seconds and checking on payment...\n");
                std::thread::sleep(std::time::Duration::from_secs(10));

                if let Some(channel) = node
                    .list_channels()
                    .iter()
                    .find(|c| c.channel_id == sc.channel_id)
                {
                    sc = Self::update_balances(sc, Some(channel.clone()));
                }

                println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
                println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);

                dollars_from_par = sc.stable_receiver_usd - sc.expected_usd;
                percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();

                println!(
                    "{:<25} {:>5}",
                    "Percent from par:",
                    format!("{:.2}%\n", percent_from_par)
                );

                println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);
            }
            Action::Pay => {
                println!("\nPaying the difference...\n");

                let amt = USD::to_msats(dollars_from_par, sc.latest_price);

                // let result = node.bolt12_payment().send_using_amount(
                //     &sc.counterparty_offer,
                //     amt,
                //     None,
                //     Some("here ya go".to_string()),
                // );

                // This is keysend / spontaneous payment code you can use if Bolt12 doesn't work

                // First, ensure we are connected
                // let result = node.connect(sc.counterparty, sc.counterparty_net_address, true);

                // if let Err(e) = result {
                //     println!("Failed to connect with : {}", e);
                // } else {
                //     println!("Successfully connected.");
                // }

                let result = node
                    .spontaneous_payment()
                    .send(amt, sc.counterparty,None);
                match result {
                    Ok(payment_id) => println!("Payment sent successfully with payment ID: {}", payment_id),
                    Err(e) => println!("Failed to send payment: {}", e),
                }

            }
            Action::HighRisk => {
                println!("Risk level high. Current risk level: {}", sc.risk_level);
            }
        }
        sc
    }
    
    fn update_balances(
        mut sc: StableChannel,
        channel_details: Option<ChannelDetails>,
    ) -> StableChannel {
        let (our_balance, their_balance) = match channel_details {
            Some(channel) => {
                let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats =
                    (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                (our_balance_sats, their_balance_sats)
            }
            None => (0, 0), // Handle the case where channel_details is None
        };

        // Update balances based on whether this is a User or provider
        if sc.is_stable_receiver {
            sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
            sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
            sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
        } else {
            sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
            sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
        }

        sc // Return the modified StableChannel
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let _connected = self.user.connect(
            PublicKey::from_str("024fa3625dbcf4511e5d0b28ec3cf590eb8bf31fc4d3a7dc3fa282a5ce4ecd6623")
                .unwrap(),
            SocketAddress::from_str("127.0.0.1:9735").unwrap(),
            true,
        );

        let result = self.user.bolt11_payment().receive_via_jit_channel(
            50_000_000,
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

    fn list_channels(&mut self) {
        let channels = self.user.list_channels();
        if channels.is_empty() {
            self.channel_list_string = "No channels found.".to_string();
        } else {
            let mut info = String::new();
            info.push_str("User Channels:\n");
            for channel in channels.iter() {
                info.push_str("--------------------------------------------\n");
                info.push_str(&format!("Channel ID: {}\n", channel.channel_id));
                info.push_str(&format!("Channel Value: {} sats\n", channel.channel_value_sats));
                info.push_str(&format!("Channel Ready?: {}\n", channel.is_channel_ready));
            }
            info.push_str("--------------------------------------------\n");
            self.channel_list_string = info;
        }
        self.channel_list = channels;
    }

    fn close_channels_to_address(&mut self) {
        // Close all channels
        for channel in self.user.list_channels().iter() {
            let user_channel_id = channel.user_channel_id;
            let counterparty_node_id = channel.counterparty_node_id;
            let _ = self.user.close_channel(&user_channel_id, counterparty_node_id);
        }

        // Withdraw everything to address
        let address_str = &self.close_channel_address;
        match Address::from_str(address_str) {
            Ok(addr) => match addr.require_network(self.network) {
                Ok(addr_checked) => {
                    match self.user.onchain_payment().send_all_to_address(&addr_checked) {
                        Ok(txid) => println!("{}", txid),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                Err(_) => eprintln!("Invalid address for this network"),
            },
            Err(_) => eprintln!("Invalid address"),
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Stable Channels ⚖️💵⚡");

                if self.user_data.is_onboarding {
                    self.list_channels();
                    if !self.channel_list.is_empty() {
                        self.user_data.waiting_for_invoice_payment = false;
                        self.user_data.is_onboarding = false;
                    }

                    if !self.user_data.waiting_for_invoice_payment && !self.user_data.has_paid_initial_invoice {
                        if ui.button("create a $100 stable channel").clicked() {
                            self.get_jit_invoice(ctx);
                            self.user_data.waiting_for_invoice_payment = true;
                        }
                    } else if self.user_data.waiting_for_invoice_payment {
                        if let Some(ref qr) = self.qr_texture {
                            ui.image(qr, qr.size_vec2());
                        } else {
                            ui.label("Big white box (placeholder)");
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.invoice_result)
                                .hint_text("Invoice..."),
                        );
                        if ui.button("Copy Invoice").clicked() {
                            ctx.output_mut(|o| {
                                o.copied_text = self.invoice_result.clone();
                            });
                        }
                        self.dot_counter = (self.dot_counter + 1) % 7;
                        let dots = ".".repeat(self.dot_counter);
                        ui.label(dots);

                        if ui.button("Check Channels").clicked() {
                            self.list_channels();
                            if !self.channel_list.is_empty() {
                                self.user_data.waiting_for_invoice_payment = false;
                                self.user_data.is_onboarding = false;
                            }
                        }
                        ui.label(&self.channel_list_string);

                        if ui.button("Back").clicked() {
                            self.user_data.waiting_for_invoice_payment = false;
                        }
                    }
                } else {
                    let now = Instant::now();
                    if now.duration_since(self.last_stability_check) >= Duration::from_secs(30) {
                        self.stable_channel = Self::check_stability(&self.user, self.stable_channel.clone());
                        self.last_stability_check = now;
                    }
                    let balances = self.user.list_balances();
                    let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                    let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                    ui.label(format!("On-Chain Balance: {}", onchain_balance));
                    ui.label(format!("Lightning Balance: {}", lightning_balance));
                    if ui.button("List Channels").clicked() {
                        self.list_channels();
                    }
                    ui.label(&self.channel_list_string);

                    // Address entry + close channels button
                    ui.text_edit_singleline(&mut self.close_channel_address);
                    if ui.button("Close channel to address").clicked() {
                        self.close_channels_to_address();
                    }

                    if ui.button("Back").clicked() {
                        self.user_data.is_onboarding = true;
                    }
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
