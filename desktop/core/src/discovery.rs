//! Finding other PCs on the local network (mDNS / DNS-SD), and being found.
//!
//! What this PC announces: its name, its certificate fingerprint, and the port of its PC-facing
//! server. Nothing about paired phones, files, notes or other PCs. The announcement can be switched
//! off completely (`hidden`); this PC can then still look for others.

use std::net::IpAddr;
use std::sync::{Arc, Weak};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::error::{Error, Result};
use crate::state::Shared;

const SERVICE_TYPE: &str = "_bridgepc._tcp.local.";
const TXT_VERSION: &str = "1";

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    registered: std::sync::Mutex<Option<String>>,
}

fn instance_of(fp: &str) -> String {
    fp[..32.min(fp.len())].to_string()
}

impl Discovery {
    pub(crate) fn start(shared: &Arc<Shared>) -> Result<Discovery> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::Invalid(format!("mDNS: {e}")))?;
        let rx = daemon.browse(SERVICE_TYPE).map_err(|e| Error::Invalid(format!("mDNS: {e}")))?;
        let weak: Weak<Shared> = Arc::downgrade(shared);
        std::thread::Builder::new()
            .name("bridge-mdns".into())
            .spawn(move || {
                while let Ok(ev) = rx.recv() {
                    let Some(s) = weak.upgrade() else { return };
                    match ev {
                        ServiceEvent::ServiceResolved(info) => {
                            let (Some(fp), Some(name)) = (info.get_property_val_str("fp"), info.get_property_val_str("n")) else { continue };
                            if info.get_property_val_str("v") != Some(TXT_VERSION) {
                                continue;
                            }
                            let addrs: Vec<IpAddr> = info.get_addresses_v4().into_iter().map(IpAddr::V4).collect();
                            s.upsert_nearby(&fp.to_ascii_lowercase(), name, addrs, info.get_port());
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            if let Some(instance) = fullname.split('.').next() {
                                s.remove_nearby_instance(instance);
                            }
                        }
                        _ => {}
                    }
                }
            })
            .map_err(Error::Io)?;
        Ok(Discovery { daemon, registered: std::sync::Mutex::new(None) })
    }

    /// (Re)publish this PC, or withdraw it when `hidden`.
    pub(crate) fn refresh(&self, name: &str, fp: &str, port: u16, hidden: bool) {
        let mut reg = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(full) = reg.take() {
            let _ = self.daemon.unregister(&full);
        }
        if hidden {
            return;
        }
        let instance = instance_of(fp);
        let host = format!("bridge-{instance}.local.");
        let props = [("n", name), ("fp", fp), ("v", TXT_VERSION)];
        match ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, &props[..]) {
            Ok(info) => {
                let info = info.enable_addr_auto();
                let full = info.get_fullname().to_string();
                match self.daemon.register(info) {
                    Ok(()) => *reg = Some(full),
                    Err(e) => tracing::warn!("could not announce this PC: {e}"),
                }
            }
            Err(e) => tracing::warn!("could not announce this PC: {e}"),
        }
    }

    pub(crate) fn shutdown(&self) {
        let mut reg = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(full) = reg.take() {
            let _ = self.daemon.unregister(&full);
        }
        let _ = self.daemon.shutdown();
    }
}
