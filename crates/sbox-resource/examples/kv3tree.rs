use sbox_resource::{kv3, Resource, Value};

fn show(v: &Value, ind: usize, out: &mut String) {
    let pad = "  ".repeat(ind);
    match v {
        Value::Object(o) => {
            out.push_str("{\n");
            for (k, v) in o {
                out.push_str(&format!("{pad}  {k} = "));
                show(v, ind + 1, out);
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        Value::Array(a) if a.iter().all(|x| !matches!(x, Value::Object(_) | Value::Array(_))) => out.push_str(&format!("{a:?}\n")),
        Value::Array(a) => {
            out.push_str("[\n");
            for v in a {
                out.push_str(&format!("{pad}  "));
                show(v, ind + 1, out);
            }
            out.push_str(&format!("{pad}]\n"));
        }
        v => out.push_str(&format!("{v:?}\n")),
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let block = std::env::args().nth(2).unwrap_or("DATA".into());
    let data = std::fs::read(&path)?;
    let res = Resource::parse(&data)?;
    let v = kv3::parse(res.block_data(&block).unwrap())?;
    let mut s = String::new();
    show(&v, 0, &mut s);
    print!("{s}");
    Ok(())
}
