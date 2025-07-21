use rollup_node_tests::*;

fn main() {
    let result = test::test_main(
        &std::env::args().collect::<Vec<_>>(),
        None,
        |_| {},
        || {}
    );
    
    if let Err(e) = result {
        eprintln!("Test failed: {:?}", e);
        std::process::exit(1);
    }
}
