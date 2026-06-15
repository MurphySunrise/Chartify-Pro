fn main() {
    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/chartify.ico");
        resource.compile().expect("failed to embed Windows icon");
    }
}
