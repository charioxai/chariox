//! One decimal quote for the person approving a machine and the agent choosing one.
use super::*;

pub(super) fn approval_quote(
    request: &ControlPlaneRequest,
    capability: Option<&MachineAllocationCapability>,
) -> Result<String, DaemonError> {
    let description = match (request, capability) {
            (
                ControlPlaneRequest::AcquireJob {
                    needs, timeout_s, ..
                },
                Some(capability),
            ) => {
                let class = super::super::job_compute::select_job_class(capability, needs)?;
                let minutes = timeout_s.saturating_add(59) / 60;
                format!("Run this job on a VM ({}, {} CPUs, {} MiB) at {}? The command limit is {minutes} minutes; charges continue through startup and until deletion.", class.class_id, class.vcpu, class.usable_memory_mib, rate_text(&class.rate_per_minute))
            }
            (
                ControlPlaneRequest::Acquire {
                    class_id: Some(class_id),
                    region,
                    ..
                },
                Some(capability),
            ) => {
                // The service validates the choice; the card names what it lists, or the bare request.
                match capability.offers.iter().find(|option| {
                    &option.class_id == class_id
                        && region.as_ref().is_none_or(|region| &option.region == region)
                }) {
                    Some(option) => format!(
                        "Acquire {} in {} ({} CPUs, {} GB) at {}? The total depends on how long you keep it.",
                        option.class_id, option.region, option.vcpu, option.ram_gb,
                        rate_text(&option.rate_per_minute)
                    ),
                    None => format!(
                        "Acquire {class_id}{} at the connected service's quoted rate?",
                        region.as_ref().map(|region| format!(" in {region}")).unwrap_or_default()
                    ),
                }
            }
            (_, Some(capability)) => capability
                .offer
                .as_ref()
                .map(|offer| {
                    if let Some(rate) = &offer.rate_per_minute {
                        return format!(
                            "Acquire {} ({} CPUs, {} GB) at {}? The total depends on how long you keep it.",
                            offer.machine_type, offer.vcpu, offer.ram_gb, rate_text(rate)
                        );
                    }
                    format!(
                        "Acquire {} ({} CPUs, {} GB) for {} {} per {:?}?",
                        offer.machine_type,
                        offer.vcpu,
                        offer.ram_gb,
                        offer.terms.amount,
                        offer.terms.currency,
                        offer.terms.period
                    )
                })
                .unwrap_or_else(|| {
                    "Acquire a machine at the connected service's quoted rate?".into()
                }),
            _ => {
                return Err(acquisition_error(
                    "capacity_unavailable",
                    "The connected service has no machine offer, so this try was not sent",
                ))
            }
        };
    Ok(description)
}

fn money(amount_micros: u128) -> String {
    format!(
        "{}.{:06}",
        amount_micros / 1_000_000,
        amount_micros % 1_000_000
    )
    .trim_end_matches('0')
    .trim_end_matches('.')
    .to_owned()
}

#[derive(serde::Serialize)]
struct DecimalRate<'a> {
    per_minute: String,
    per_hour: String,
    currency: &'a str,
}

fn decimal_rate(rate: &crate::local::JobMachineRate) -> DecimalRate<'_> {
    let micros = u128::from(rate.amount_micros);
    DecimalRate {
        per_minute: money(micros),
        per_hour: money(micros * 60),
        currency: &rate.currency,
    }
}

fn rate_text(rate: &crate::local::JobMachineRate) -> String {
    let DecimalRate {
        per_minute,
        per_hour,
        currency,
    } = decimal_rate(rate);
    format!("{per_minute} {currency} per minute ({per_hour} {currency} per hour)")
}

pub(super) fn machine_options(capability: &MachineAllocationCapability) -> serde_json::Value {
    let options = capability
        .offers
        .iter()
        .map(|offer| {
            let mut value = serde_json::json!(offer);
            value["rate"] = serde_json::json!(decimal_rate(&offer.rate_per_minute));
            value
        })
        .collect::<Vec<_>>();
    let default = capability.offer.as_ref().map(|offer| {
        let mut value = serde_json::json!(offer);
        if let Some(rate) = &offer.rate_per_minute {
            value["rate"] = serde_json::json!(decimal_rate(rate));
        }
        value
    });
    serde_json::json!({"options":options,"default":default})
}
