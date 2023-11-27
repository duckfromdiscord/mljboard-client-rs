use base64::{engine::general_purpose, Engine as _};
use clap::{Arg, Command};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use log::warn;
use mljboard_client::json::{HOSClientReq, HOSServerReq};
use reqwest::Client;
use tokio::{
    net::TcpStream,
    time::{sleep, Duration},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    {connect_async, tungstenite::protocol::Message},
};

fn get_client() -> Client {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
}

async fn handle_message(
    message: Message,
    local_addr: String,
    pairing_code: Option<String>,
    write: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
) {
    let raw_json = String::from_utf8(message.into_data()).unwrap();
    match serde_json::from_str::<HOSServerReq>(&raw_json) {
        Ok(serverreq) => {
            dbg!(serverreq.clone());
            match serverreq.method.as_str() {
                "GET" => {
                    let response = get_client()
                        .get(local_addr.clone() + "/" + &serverreq.url)
                        .send()
                        .await;
                    match response {
                        Ok(response) => {
                            let _ = write
                                .send(Message::Text(
                                    serde_json::to_string(&HOSClientReq {
                                        _type: "response".to_string(),
                                        id: serverreq.id,
                                        code: pairing_code.clone(),
                                        status: Some(response.status().as_u16()),
                                        content: Some(
                                            general_purpose::STANDARD
                                                .encode(response.text().await.unwrap()),
                                        ),
                                    })
                                    .unwrap(),
                                ))
                                .await;
                        }
                        Err(err) => {
                            log::error!("Error in HTTP response from local server: {}", err);
                        }
                    }
                }
                _ => (),
            }
        }
        Err(err) => {
            warn!("Bad server request. {}", err);
        }
    }
}

#[tokio::main]
async fn main() {
    let matches = Command::new("mljboard-client")
        .arg(Arg::new("haddr").short('a').value_name("HADDR").help(
            "HOS address, including `ws://` or `wss://` and the path (e.g. ws://127.0.0.1:9003/ws)",
        ))
        .arg(
            Arg::new("laddr")
                .short('l')
                .value_name("LADDR")
                .help("Local address to forward (e.g. http://127.0.0.1:42010/)"),
        )
        .arg(
            Arg::new("pairing_code")
                .short('c')
                .value_name("PAIRING_CODE")
                .help("HOS pairing code"),
        )
        .get_matches();

    let hos_addr: String = matches
        .get_one::<String>("haddr")
        .expect("HOS server address required")
        .to_string();
    let local_addr: String = matches
        .get_one::<String>("laddr")
        .expect("Local address to forward required")
        .to_string()
        .trim_end_matches('/')
        .to_string();
    let pairing_code: Option<String> = matches
        .get_one::<String>("pairing_code")
        .map(|x| Some(x.clone()))
        .unwrap_or(None);

    let pairing_msg = serde_json::to_string(&HOSClientReq {
        _type: "pairing".to_string(),
        id: None,
        code: pairing_code.clone(),
        status: None,
        content: None,
    })
    .unwrap();

    let url = url::Url::parse(&hos_addr).unwrap();

    loop {
        match connect_async(url.clone()).await {
            Ok((ws_stream, _response)) => {
                println!("Connected to HOS server");
                let (mut write, mut read) = ws_stream.split();
                write
                    .send(Message::Text(pairing_msg.clone() + "\n"))
                    .await
                    .unwrap();

                loop {
                    match read.next().await {
                        Some(maybe_message) => match maybe_message {
                            Ok(message) => {
                                handle_message(
                                    message,
                                    local_addr.clone(),
                                    pairing_code.clone(),
                                    &mut write,
                                )
                                .await;
                            }
                            Err(_error) => {
                                println!("Disconnected from HOS server. Retrying.");
                                break;
                            }
                        },
                        None => {}
                    }
                }
            }
            Err(err) => {
                println!("Failed to connect to HOS server. Retrying in 3s.");
                dbg!("{}", err);
                sleep(Duration::new(3, 0)).await;
            }
        };
    }
}
