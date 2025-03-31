use oci_client::Reference;
use ptsession::PtSession;

pub async fn oras_pull(
    reference: Reference,
    session: PtSession,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
