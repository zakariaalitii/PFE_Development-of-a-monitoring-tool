use serde::Deserialize;

#[repr(u8)]
#[derive(Deserialize)]
pub enum Resource {
    ALL,
    CPU,
    MEM,
    STOCKAGE
}

#[derive(Deserialize)]
pub struct UsageInfo {
    from: u64,
    num: usize,
    res: Resource
}
