use ptsession::PtSession;
use serde_json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = PtSession::from("tests/RegionTest.ptx");
    let bytes = serde_json::to_vec_pretty(&session)?;
    std::fs::write("tests/MarkerTest.json", bytes)?;
    Ok(())
}
