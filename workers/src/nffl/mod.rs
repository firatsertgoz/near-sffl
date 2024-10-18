use blsful::{Bls12381G1Impl, BlsError, SecretKey, Signature, SignatureSchemes};

/// Return the BLS signature from NFFL verifying the message.
///
/// NOTE: mocking this for now by returning just a BLS signature,
/// will include correct implementation later.
///
pub fn nffl_verify() -> Result<Signature<Bls12381G1Impl>, BlsError> {
    let sk = SecretKey::<Bls12381G1Impl>::from_hash(b"seed phrase");
    sk.sign(SignatureSchemes::Basic, b"message to sign")
}
