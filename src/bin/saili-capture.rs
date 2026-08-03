use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use saili::{ReadReportStatus, SailiDevice};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Capture raw SAILI HID reports without TUI or backend work"
)]
struct Arguments {
    #[arg(long)]
    count: Option<u64>,

    #[arg(long)]
    duration: Option<f64>,

    #[arg(long)]
    csv: Option<PathBuf>,

    #[arg(long)]
    include_unchanged: bool,
}

fn main() -> Result<(), CaptureError> {
    let arguments = Arguments::parse();
    let device = SailiDevice::connect().map_err(CaptureError::Device)?;
    let mut output = Output::new(arguments.csv.as_ref()).map_err(CaptureError::Output)?;
    write_metadata(&mut output, device.identity()).map_err(CaptureError::Output)?;
    writeln!(
        output.writer,
        "sequence,timestamp_us,interval_us,length,bytes_hex,changed_mask"
    )
    .map_err(CaptureError::Output)?;

    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut previous: Option<(Instant, Vec<u8>)> = None;
    loop {
        if arguments.count.is_some_and(|count| sequence >= count)
            || arguments
                .duration
                .is_some_and(|seconds| started.elapsed().as_secs_f64() >= seconds)
        {
            break;
        }

        match device
            .read_report(Duration::from_millis(10))
            .map_err(CaptureError::Device)?
        {
            ReadReportStatus::Timeout => {}
            ReadReportStatus::Report { bytes, received_at } => {
                sequence = sequence.saturating_add(1);
                let interval_us = previous.as_ref().map_or(0, |(previous_at, _)| {
                    received_at
                        .saturating_duration_since(*previous_at)
                        .as_micros()
                });
                let changed_mask =
                    changed_mask(previous.as_ref().map(|(_, bytes)| bytes.as_slice()), &bytes);
                if arguments.include_unchanged || changed_mask != 0 {
                    writeln!(
                        output.writer,
                        "{}",
                        csv_record(
                            sequence,
                            received_at.saturating_duration_since(started).as_micros(),
                            interval_us,
                            &bytes,
                            changed_mask,
                        )
                    )
                    .map_err(CaptureError::Output)?;
                }
                previous = Some((received_at, bytes));
            }
        }
    }
    output.writer.flush().map_err(CaptureError::Output)?;
    Ok(())
}

fn changed_mask(previous: Option<&[u8]>, current: &[u8]) -> u8 {
    let Some(previous) = previous else {
        return u8::MAX;
    };
    let mut mask = 0_u8;
    for index in 0..current.len().min(previous.len()).min(8) {
        if current[index] != previous[index] {
            mask |= 1 << index;
        }
    }
    if current.len() != previous.len() {
        mask = u8::MAX;
    }
    mask
}

fn csv_record(
    sequence: u64,
    timestamp_us: u128,
    interval_us: u128,
    bytes: &[u8],
    changed_mask: u8,
) -> String {
    let bytes_hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{sequence},{timestamp_us},{interval_us},{},{bytes_hex},{changed_mask:02x}",
        bytes.len()
    )
}

fn write_metadata(output: &mut Output, identity: &saili::DeviceIdentity) -> io::Result<()> {
    writeln!(output.writer, "# vendor_id=0x1781")?;
    writeln!(output.writer, "# product_id=0x0898")?;
    writeln!(output.writer, "# manufacturer={}", identity.manufacturer)?;
    writeln!(output.writer, "# product={}", identity.product)?;
    writeln!(
        output.writer,
        "# serial={}",
        identity.serial_number.as_deref().unwrap_or("--")
    )?;
    writeln!(output.writer, "# path={}", identity.path)?;
    writeln!(output.writer, "# usage_page=0x{:04x}", identity.usage_page)?;
    writeln!(output.writer, "# usage=0x{:04x}", identity.usage)?;
    writeln!(output.writer, "# interface={}", identity.interface_number)?;
    writeln!(
        output.writer,
        "# descriptor_hash={}",
        identity.descriptor_hash.as_deref().unwrap_or("--")
    )?;
    writeln!(
        output.writer,
        "# kernel_driver={}",
        identity.kernel_driver.as_deref().unwrap_or("--")
    )?;
    Ok(())
}

struct Output {
    writer: Box<dyn Write>,
}

impl Output {
    fn new(path: Option<&PathBuf>) -> io::Result<Self> {
        let writer: Box<dyn Write> = match path {
            Some(path) => Box::new(BufWriter::new(File::create(path)?)),
            None => Box::new(BufWriter::new(io::stdout())),
        };
        Ok(Self { writer })
    }
}

#[derive(Debug, thiserror::Error)]
enum CaptureError {
    #[error(transparent)]
    Device(#[from] saili::SailiError),

    #[error("capture output failed")]
    Output(#[source] io::Error),
}

#[cfg(test)]
mod tests {
    use super::{changed_mask, csv_record};

    #[test]
    fn changed_mask_marks_first_and_changed_bytes() {
        assert_eq!(changed_mask(None, &[1, 2]), 0xff);
        assert_eq!(changed_mask(Some(&[1, 2]), &[1, 2]), 0);
        assert_eq!(changed_mask(Some(&[1, 2]), &[1, 3]), 0x02);
        assert_eq!(changed_mask(Some(&[1]), &[1, 2]), 0xff);
    }

    #[test]
    fn csv_record_contains_timing_length_bytes_and_mask() {
        assert_eq!(
            csv_record(4, 123, 10, &[0x01, 0xab], 0x02),
            "4,123,10,2,01 ab,02"
        );
    }
}
