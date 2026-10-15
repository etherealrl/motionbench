use super::*;

#[test]
fn only_the_variant_means_cancelled() {
    assert_ne!(Error::from("analysis cancelled"), Error::Cancelled);
    let asked = AtomicBool::new(true);
    assert_eq!(check(&asked), Err(Error::Cancelled));
    assert_eq!(Error::from(check_io(&asked).unwrap_err()), Error::Cancelled);
    assert!(check(&AtomicBool::new(false)).is_ok());
    assert_eq!(
        Error::from(std::io::Error::other("analysis cancelled")),
        Error::Failed("analysis cancelled".into())
    );
    assert_eq!(Error::Cancelled.to_string(), "analysis cancelled");
}
