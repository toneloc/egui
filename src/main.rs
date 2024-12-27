use eframe::{egui, App, Frame};
use egui::{TextureHandle, TextureOptions};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network},
    Builder,
};
use hex;
use image::{GrayImage, Luma};
use qrcode::{Color, QrCode};

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
    qr_texture: Option<TextureHandle>,
}

fn make_node(alias: &str, port: u16, lsp_pubkey: Option<PublicKey>) -> ldk_node::Node {
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
        let bytes = hex::decode("031a0122af946d4d59e2c5d4b98bb4ae1a38ff7478bd7386da0df0c68e5e127281")
            .unwrap();
        let lsp_pubkey = PublicKey::from_slice(&bytes).ok();
        let user = make_node("user", 9735, lsp_pubkey);

        Self {
            user_data: UserData::default(),
            invoice_result: String::new(),
            user,
            qr_texture: None,
        }
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
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

                let mut imgbuf = GrayImage::new(
                    (width * scale_factor) as u32,
                    (width * scale_factor) as u32,
                );

                // Scale QR code
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

                // Convert to RGBA
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for pixel in imgbuf.pixels() {
                    let lum = pixel[0];
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(255);
                }
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.qr_texture = Some(ctx.load_texture(
                    "qr_code",
                    color_image,
                    TextureOptions::LINEAR,
                ));
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
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
                            self.get_jit_invoice(ctx);
                            self.user_data.waiting_for_onboarding = true;
                        }
                    } else {
                        // Show QR code
                        if let Some(ref qr) = self.qr_texture {
                            ui.image(qr, qr.size_vec2());
                        }

                        // Editable text box for invoice, so user can copy/paste
                        ui.add(egui::TextEdit::singleline(&mut self.invoice_result)
                            .hint_text("Invoice..."));

                        // Button to copy the invoice to clipboard
                        if ui.button("Copy Invoice").clicked() {
                            ctx.output_mut(|o| {
                                o.copied_text = self.invoice_result.clone();
                            });
                        }

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
