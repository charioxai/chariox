wit_bindgen::generate!({ world: "probe", path: "../wit" });

struct Probe;
export!(Probe);

impl Guest for Probe {
    async fn run(n: u32) -> String {
        let mut last = String::new();
        for i in 0..n {
            last = spike::probe::kernel::get(format!("key-{i}")).await;
        }
        last
    }
}
