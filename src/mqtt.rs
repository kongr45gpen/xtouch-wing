use std::time::Duration;

use log::{debug, error};
use rumqttc::{AsyncClient, MqttOptions};
use tokio::{task, time};

pub struct Mqtt {}

impl Mqtt {
    pub async fn new(remote_host: &str, remote_port: u16) -> anyhow::Result<Self> {
        let mut mqttoptions = MqttOptions::new("xtouch-wing-client", remote_host, remote_port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));

        let (mut client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

        task::spawn(async move {
            let payload = r#"{
                "dev": {
                    "ids": "xtouch_wing_001",
                    "name": "XTouch Wing",
                    "mf": "kongr45gpen",
                    "mdl": "X-Touch Wing",
                    "sw": "1.0"
                },
                 "origin": {
                    "name":"xtouch-wing",
                    "sw": "1.0",
                    "url": "https://github.com/kongr45gpen/xtouch-wing"
                },
                "cmps": {
                    "main_volume": {
                        "p": "number",
                        "device_class": "sound_pressure",
                        "unit_of_measurement": "%",
                        "min": 0,
                        "max": 100,
                        "unique_id": "xtw01_main_vol",
                        "name": "Volume",
                        "icon": "mdi:volume-high",
                        "value_template": "{{ value_json.main_volume }}"
                    }
                },
                "command_topic": "xtouchwing/command",
                "state_topic": "xtouchwing/state",
                "qos": 2
            }"#;

            let result = client
                .publish(
                    "homeassistant/device/xtouchwing/config",
                    rumqttc::QoS::AtLeastOnce,
                    true,
                    payload,
                )
                .await;

            if let Err(e) = result {
                error!("Failed to publish MQTT config: {:?}", e);
            }

            let result = client
                .publish(
                    "xtouchwing/state",
                    rumqttc::QoS::AtLeastOnce,
                    false,
                    r#"{ "main_volume": 50 }"#,
                )
                .await;

            if let Err(e) = result {
                error!("Failed to publish MQTT config: {:?}", e);
            }

            let result = client
                .subscribe("xtouchwing/command", rumqttc::QoS::ExactlyOnce)
                .await;

            if let Err(e) = result {
                error!("Failed to subscribe to MQTT command topic: {:?}", e);
            }

            loop {
                debug!("MQTT in your loop");
                while let Ok(notification) = eventloop.poll().await {
                    println!("Received = {:?} = {:?}", 1, notification);

                    if let rumqttc::Event::Incoming(incoming) = notification {
                        debug!("Received MQTT message on topic '{}': {:?}", 1, incoming);

                        if let rumqttc::Packet::Publish(publish) = incoming {
                            let topic = publish.topic;
                            let payload = String::from_utf8_lossy(&publish.payload);

                            debug!("MQTT Publish received on topic '{}': {}", topic, payload);
                        }
                    }
                }
            }
        });

        Ok(Self {})
    }
}
