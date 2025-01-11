use core::f64;
use std::str::FromStr;

use egui::{TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::bitcoin::{Address, Network};
use ldk_node::{Node, ChannelDetails};
use lightning::ln::msgs::SocketAddress;
use lightning::ln::types::ChannelId;
use lightning::routing::gossip::NodeId;
use qrcode::{Color, QrCode};
use ureq::Agent;
use crate::types::{Bitcoin, StableChannel, USD};
use crate::price_feeds::{calculate_median_price, fetch_prices, set_price_feeds};

/// Core stability logic
pub fn check_stability(node: &Node, sc: &mut StableChannel) {
    sc.latest_price = get_latest_price();

    if let Some(channel) = node
        .list_channels()
        .iter()
        .find(|c| c.channel_id == sc.channel_id)
    {
        update_balances(sc, Some(channel.clone()));
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
            // println!("\nWaiting 10 seconds and checking on payment...\n");
            // std::thread::sleep(std::time::Duration::from_secs(10));

            if let Some(channel) = node
                .list_channels()
                .iter()
                .find(|c| c.channel_id == sc.channel_id)
            {
                update_balances(sc, Some(channel.clone()));
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
}

pub fn get_latest_price() -> f64 {
    let latest_price = fetch_prices(&Agent::new(), &set_price_feeds())
        .and_then(|prices| calculate_median_price(prices))
        .unwrap_or(0.0);
    latest_price
}

pub fn update_balances(sc: &mut StableChannel, channel_details: Option<ChannelDetails>) {

    let (our_balance, their_balance) = match channel_details {
        Some(channel) => {
            let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
            let our_balance_sats =
                (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
            let their_balance_sats = channel.channel_value_sats - our_balance_sats;
            (our_balance_sats, their_balance_sats)
        }
        None => (0, 0),
    };

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
}

pub fn connect_to_lsp_and_entry_node(node: &Node) {
    let _connected_to_lsp = node.connect(
        PublicKey::from_str("025d4c41316f9d847ed3ec827751f1df4efabb6aa48c162b29f9aabf5eb148f8b1")
            .unwrap(),
        SocketAddress::from_str("127.0.0.1:9737").unwrap(),
        true,
    );

    println!("Connection result: {:?}", _connected_to_lsp.unwrap());

    let _connected_to_exchange = node.connect(
        PublicKey::from_str("02e897f0ce1bf88afe1f8e2be0045294ec87b00eebd689e42ba7290cfa2922dbe7")
            .unwrap(),
        SocketAddress::from_str("127.0.0.1:9735").unwrap(),
        true,
    );

    println!("Connection result: {:?}", _connected_to_exchange.unwrap());

    let node_info = node
        .network_graph()
        .node(&NodeId::from_pubkey(
            &PublicKey::from_str("025d4c41316f9d847ed3ec827751f1df4efabb6aa48c162b29f9aabf5eb148f8b1")
                .unwrap(),
        ));

    println!("Node information: {:?}", node_info);

    let node_info = node
        .network_graph()
        .node(&NodeId::from_pubkey(
            &PublicKey::from_str("02e897f0ce1bf88afe1f8e2be0045294ec87b00eebd689e42ba7290cfa2922dbe7")
                .unwrap(),
        ));

    println!("Node information: {:?}", node_info);
}

pub fn list_channels(node: &Node) -> (Vec<ChannelDetails>, String) {
    let channels = node.list_channels();
    let mut info = String::new();

    if channels.is_empty() {
        info.push_str("No channels found.");
    } else {
        info.push_str("User Channels:\n");
        for channel in &channels {
            info.push_str("--------------------------------------------\n");
            info.push_str(&format!("Channel ID: {}\n", channel.channel_id));
            info.push_str(&format!("Channel Value: {} sats\n", channel.channel_value_sats));
            info.push_str(&format!("Channel Ready?: {}\n", channel.is_channel_ready));
        }
        info.push_str("--------------------------------------------\n");
    }

    (channels, info)
}

pub fn close_channels_to_address(node: &Node, address_str: String) {
    for channel in node.list_channels().iter() {
        let user_channel_id = channel.user_channel_id;
        let counterparty_node_id = channel.counterparty_node_id;
        let _ = node.close_channel(&user_channel_id, counterparty_node_id);
    }

    // Withdraw everything to address
    match Address::from_str(&address_str) {

        Ok(addr) => match addr.require_network(Network::Signet,) {
            Ok(addr_checked) => {
                match node.onchain_payment().send_all_to_address(&addr_checked) {
                    Ok(txid) => println!("{}", txid),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            Err(_) => eprintln!("Invalid address for this network"),
        },
        Err(_) => eprintln!("Invalid address"),
    }
}