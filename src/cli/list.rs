use oci_client::Reference;
use ptsession::PtSession;

pub async fn oras_info(
    reference: Reference,
    session: PtSession,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
