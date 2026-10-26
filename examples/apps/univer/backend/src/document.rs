use serde_json::Value;

pub const MAX_BYTES: usize = 192 * 1024;

pub fn workbook(value: &Value) -> Result<(), String> {
    if value.to_string().len() > MAX_BYTES {
        return Err("workbook exceeds 192 KiB; export or split the workbook".into());
    }
    let order = value["sheetOrder"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 16);
    let sheets = value["sheets"].as_object();
    let (Some(order), Some(sheets)) = (order, sheets) else {
        return Err("workbook needs sheetOrder and sheets".into());
    };
    if value["id"].as_str().is_none_or(str::is_empty)
        || !value["name"].is_string()
        || sheets.len() != order.len()
    {
        return Err("invalid workbook identity or sheet count".into());
    }
    let mut seen = std::collections::HashSet::new();
    for id in order {
        let id = id
            .as_str()
            .filter(|id| seen.insert(*id))
            .ok_or("invalid or repeated sheet id")?;
        let sheet = sheets.get(id).ok_or("sheet is missing")?;
        let rows = sheet["rowCount"]
            .as_u64()
            .filter(|v| (1..=10000).contains(v));
        let columns = sheet["columnCount"]
            .as_u64()
            .filter(|v| (1..=256).contains(v));
        let (Some(rows), Some(columns), Some(cells)) =
            (rows, columns, sheet["cellData"].as_object())
        else {
            return Err("invalid worksheet dimensions or cell data".into());
        };
        if sheet["id"] != id || !sheet["name"].is_string() {
            return Err("invalid worksheet identity".into());
        }
        for (row, data) in cells {
            if !index(row, rows) {
                return Err("invalid row index".into());
            }
            for (col, cell) in data.as_object().ok_or("invalid cell row")? {
                if !index(col, columns) || !(cell.is_object() || cell.is_null()) {
                    return Err("invalid column index or cell".into());
                }
            }
        }
    }
    Ok(())
}

fn index(text: &str, limit: u64) -> bool {
    text.parse::<u64>()
        .is_ok_and(|v| v < limit && v.to_string() == text)
}

pub fn filename(value: &Value) -> Result<String, String> {
    let name = value.as_str().ok_or("name must be a filename")?;
    if name.is_empty()
        || name.len() > 80
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || !(name.ends_with(".csv") || name.ends_with(".univer.json"))
    {
        return Err("use a plain .csv or .univer.json filename of at most 80 characters".into());
    }
    Ok(name.into())
}

pub fn file(name: &str, text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES + 128 {
        return Err("file exceeds 192 KiB".into());
    }
    if name.ends_with(".univer.json") {
        let value: Value = serde_json::from_str(text).map_err(|_| "file is not JSON")?;
        if value["format"] != "chariox-univer-workbook" || value["version"] != 1 {
            return Err("open a Chariox Univer JSON workbook".into());
        }
        workbook(&value["workbook"])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn private_file_names_never_address_a_host_path() {
        for name in [
            "../x.csv",
            "/tmp/x.csv",
            ".csv",
            "x.xlsx",
            "a/b.csv",
            "a\\b.csv",
        ] {
            assert!(filename(&json!(name)).is_err(), "{name}");
        }
        assert_eq!(
            filename(&json!("Budget.univer.json")).unwrap(),
            "Budget.univer.json"
        );
    }

    #[test]
    fn workbook_file_retains_formula_and_refuses_invalid_dimensions() {
        let mut book = json!({"id":"book","name":"Budget","sheetOrder":["s"],"sheets":{
            "s":{"id":"s","name":"Sheet1","rowCount":10,"columnCount":4,
            "cellData":{"0":{"0":{"f":"=B1+C1","v":5},"1":{"v":2},"2":{"v":3}}}}
        }});
        let text =
            json!({"format":"chariox-univer-workbook","version":1,"workbook":book}).to_string();
        file("Budget.univer.json", &text).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["workbook"],
            book
        );
        book["sheets"]["s"]["rowCount"] = json!(10001);
        assert!(workbook(&book).is_err());
        book["sheets"]["s"]["rowCount"] = json!(10);
        book["sheets"]["s"]["cellData"]["01"] = json!({});
        assert!(workbook(&book).is_err());
    }
}
