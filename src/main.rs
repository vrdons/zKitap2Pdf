pub mod paths;
pub mod utils;

#[tokio::main]
async fn main() {
    let files = utils::filesystem::check_input().unwrap_or_else(|e| {
        panic!("check_input hata verdi: {}", e);
    });
    if files.is_empty() {
        panic!("check_input: hiçbir dosya bulunamadı!");
    }
    for item in files {
        utils::upx::decompress(&item.path).await.unwrap_or_else(|e| {
             panic!("decompress hata verdi: {}", e);
        });
        panic!("{:?}", item);
    }
}
