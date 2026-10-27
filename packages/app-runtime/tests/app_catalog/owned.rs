use super::*;
use std::sync::Arc;

#[test]
fn owned_call_retains_the_exact_catalog_and_rechecks_revocation_on_reply() {
    let package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "owned", &package, &trust);
    let catalog = Arc::new(package.catalog(&mut connection, &token, &trust).unwrap());
    let name = catalog.tools().next().unwrap().name.clone();
    let tx = connection.transaction().unwrap();
    let call = catalog
        .prepare(
            &tx,
            "owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "owned-call",
            10,
            100,
        )
        .unwrap()
        .into_owned(catalog.clone())
        .unwrap();
    assert!(Arc::ptr_eq(call.catalog(), &catalog));
    tx.commit().unwrap();
    drop(catalog);
    PublisherTrustRegistry::new(&mut connection)
        .revoke(
            "owner",
            "com.example",
            "developer",
            trust.revision(),
            &decision("revoke"),
            20,
        )
        .unwrap();
    let tx = connection.transaction().unwrap();
    assert!(call
        .accept(
            &tx,
            "owner",
            response("owned-call", token.generation, json!({"text":"ok"})),
            21
        )
        .is_err());
}

#[test]
fn owned_conversion_rejects_another_catalog_object_and_accepts_a_correlated_reply() {
    let package = Package::new();
    let (mut connection, trust) = database(&package);
    let token = install(&mut connection, "owned", &package, &trust);
    let catalog = Arc::new(package.catalog(&mut connection, &token, &trust).unwrap());
    let another = Arc::new(package.catalog(&mut connection, &token, &trust).unwrap());
    let name = catalog.tools().next().unwrap().name.clone();
    let tx = connection.transaction().unwrap();
    let call = catalog
        .prepare(
            &tx,
            "owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "owned-call",
            10,
            100,
        )
        .unwrap();
    assert!(matches!(
        call.into_owned(another),
        Err(CatalogError::Provenance)
    ));
    let call = catalog
        .prepare(
            &tx,
            "owner",
            &name,
            json!({"text":"ok"}),
            &context(),
            "owned-call",
            10,
            100,
        )
        .unwrap()
        .into_owned(catalog.clone())
        .unwrap();
    assert_eq!(
        call.accept(
            &tx,
            "owner",
            response("owned-call", token.generation, json!({"text":"done"})),
            21
        )
        .unwrap(),
        json!({"text":"done"})
    );
}
