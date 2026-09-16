use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredService {
    /// Friendly kind: "propresenter", "stage", or "ndi".
    pub kind: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
}

fn kind_for(service_type: &str) -> &'static str {
    if service_type.contains("prolink") {
        "propresenter"
    } else if service_type.contains("stagedsply") {
        "stage"
    } else if service_type.contains("ndi") {
        "ndi"
    } else {
        "other"
    }
}

/// Determine the local IPv4 address the OS would use to reach the internet.
/// No packets are sent — connecting a UDP socket to an external address lets
/// the OS fill in the local side without sending anything.
fn local_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()? {
        SocketAddr::V4(a) => Some(*a.ip()),
        _ => None,
    }
}

/// Scan every host in the same /24 as `local_ip` for ProPresenter's API.
/// Tries port 1025 (default API port). Skips our own IP. Returns quickly by
/// running all probes concurrently with a short per-host timeout.
async fn subnet_scan(local_ip: Ipv4Addr) -> Vec<DiscoveredService> {
    let octets = local_ip.octets();
    let base = [octets[0], octets[1], octets[2]];
    let pp_ports: &[u16] = &[1025, 1024];

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .unwrap_or_default();

    let mut tasks = Vec::new();
    for host_octet in 1u8..=254 {
        if host_octet == octets[3] {
            continue; // skip our own IP
        }
        let ip = Ipv4Addr::new(base[0], base[1], base[2], host_octet);
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            for &port in pp_ports {
                let url = format!("http://{}:{}/version", ip, port);
                if let Ok(resp) = client.get(&url).send().await {
                    if resp.status().is_success() {
                        if let Ok(text) = resp.text().await {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                if json.get("name").is_some() || json.get("versionNumber").is_some() {
                                    let ip_str = ip.to_string();
                                    return Some(DiscoveredService {
                                        kind: "propresenter".to_string(),
                                        name: json
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("ProPresenter")
                                            .to_string(),
                                        host: ip_str.clone(),
                                        port,
                                        addresses: vec![ip_str],
                                    });
                                }
                            }
                        }
                    }
                }
            }
            None
        }));
    }

    let mut results = Vec::new();
    for t in tasks {
        if let Ok(Some(svc)) = t.await {
            results.push(svc);
        }
    }
    results
}

/// Browse the local network for ProPresenter, Stage Display and NDI services.
/// Runs mDNS discovery and, if that returns nothing, falls back to a TCP
/// subnet scan — which works in Docker where multicast is unavailable.
pub async fn discover_services(secs: Option<u64>) -> Result<Vec<DiscoveredService>, String> {
    let window = Duration::from_secs(secs.unwrap_or(4));
    let service_types = [
        "_pro7prolink._tcp.local.",
        "_pro7stagedsply._tcp.local.",
        "_ndi._tcp.local.",
    ];

    let mdns_result: Vec<DiscoveredService> = 'mdns: {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("mDNS daemon failed: {e}");
                break 'mdns vec![];
            }
        };
        let mut receivers = Vec::new();
        for st in service_types {
            match daemon.browse(st) {
                Ok(rx) => receivers.push((st, rx)),
                Err(e) => eprintln!("browse {st} failed: {e}"),
            }
        }

        let found: std::sync::Arc<tokio::sync::Mutex<HashMap<String, DiscoveredService>>> =
            std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let mut handles = Vec::new();
        for (st, rx) in receivers {
            let found = found.clone();
            let kind = kind_for(st).to_string();
            handles.push(tokio::spawn(async move {
                let _ = tokio::time::timeout(window, async {
                    while let Ok(event) = rx.recv_async().await {
                        if let ServiceEvent::ServiceResolved(info) = event {
                            let mut addresses: Vec<String> =
                                info.get_addresses().iter().map(|a| a.to_string()).collect();
                            addresses.sort_by_key(|a| usize::from(a.contains(':')));
                            let service = DiscoveredService {
                                kind: kind.clone(),
                                name: info
                                    .get_fullname()
                                    .split('.')
                                    .next()
                                    .unwrap_or(info.get_fullname())
                                    .replace('\\', ""),
                                host: info.get_hostname().trim_end_matches('.').to_string(),
                                port: info.get_port(),
                                addresses,
                            };
                            found
                                .lock()
                                .await
                                .insert(info.get_fullname().to_string(), service);
                        }
                    }
                })
                .await;
            }));
        }

        for h in handles {
            let _ = h.await;
        }
        let _ = daemon.shutdown();

        let map = found.lock().await;
        map.values().cloned().collect()
    };

    if !mdns_result.is_empty() {
        return Ok(mdns_result);
    }

    // mDNS found nothing (common in Docker where multicast is blocked).
    // Fall back to a TCP port scan of the local /24 subnet.
    if let Some(local_ip) = local_ipv4() {
        let scan = subnet_scan(local_ip).await;
        if !scan.is_empty() {
            return Ok(scan);
        }
    }

    Ok(vec![])
}

/// No-argument variant called by the web dispatch table (uses the default 4-second window).
pub async fn discover_services_core() -> Result<Vec<DiscoveredService>, String> {
    discover_services(None).await
}
