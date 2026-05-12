//! `system_monitor_disk` — disk usage (cached because mount scan isn't <1ms).

use async_trait::async_trait;
use sysinfo::Disks;

use crate::payload::{BarsData, Body, Payload, RatioData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{disk_bars, disk_label, payload, primary_disk, ratio_of};

pub struct SystemMonitorDisk;

#[async_trait]
impl Fetcher for SystemMonitorDisk {
    fn name(&self) -> &str {
        "system_monitor_disk"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Disk usage for the largest mounted volume. `Ratio` drives gauges with the used/total fraction; `Text` formats it as `\"45% of 500 GB\"`; `Bars` lists every mount with used bytes for chart_bar."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::Bars]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.58, "disk"),
            Shape::Text => samples::text("58% of 400 GB"),
            Shape::Bars => samples::bars(&[("/", 42), ("/home", 110), ("/data", 200)]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let disks = Disks::new_with_refreshed_list();
        let payload = match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Bars => payload(Body::Bars(BarsData {
                bars: disk_bars(&disks),
            })),
            Shape::Text => payload(Body::Text(TextData {
                value: primary_disk(&disks)
                    .map(|(t, a)| disk_label(t, a))
                    .unwrap_or_else(|| "no disks".into()),
            })),
            _ => primary_disk(&disks)
                .map(|(total, available)| {
                    let used = total.saturating_sub(available);
                    payload(Body::Ratio(RatioData {
                        value: ratio_of(used, total),
                        label: Some(disk_label(total, available)),
                        denominator: None,
                    }))
                })
                .unwrap_or_else(|| {
                    payload(Body::Text(TextData {
                        value: "no disks".into(),
                    }))
                }),
        };
        Ok(payload)
    }
}
