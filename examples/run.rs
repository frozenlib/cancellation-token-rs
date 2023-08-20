use cancellation_token::{CancellationToken, CancellationTokenSource, MayBeCanceled};
use rt_local::runtime::core::main;

#[main]
async fn main() {
    let cts = CancellationTokenSource::new();
    if cancelable_function(&cts.token()).await.is_err() {
        println!("canceled");
    }
}

async fn cancelable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    for _ in 0..100 {
        ct.run(heavy_work()).await?;
    }
    Ok(100)
}
async fn heavy_work() {}
