fn main() {
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Impressão de Foto A4");
        res.set("FileDescription", "Impressão de fotos em folha A4");
        res.compile().expect("falha ao embutir recursos do Windows");
    }
}
