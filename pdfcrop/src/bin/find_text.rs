use anyhow::Result;
use clap::Parser;
use lopdf::{Document, Object};
use std::collections::HashMap;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    input: String,

    #[arg(short, long)]
    page: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let doc = Document::load(&args.input)?;

    let page_id = *doc.get_pages().get(&args.page).ok_or_else(|| anyhow::anyhow!("Page not found"))?;
    println!("Page ID: {:?}", page_id);

    let page = doc.get_object(page_id)?.as_dict()?;
    let resources = match page.get(b"Resources") {
        Ok(Object::Reference(id)) => doc.get_object(*id)?.as_dict()?,
        Ok(Object::Dictionary(d)) => d,
        _ => return Ok(()),
    };
    let fonts = match resources.get(b"Font") {
        Ok(Object::Reference(id)) => doc.get_object(*id)?.as_dict()?,
        Ok(Object::Dictionary(d)) => d,
        _ => return Ok(()),
    };

    let mut font_maps = HashMap::new();

    for (name, font_ref) in fonts.iter() {
        let font_obj = doc.get_object(font_ref.as_reference()?)?.as_dict()?;
        if let Ok(to_unicode_ref) = font_obj.get(b"ToUnicode") {
            let to_unicode = doc.get_object(to_unicode_ref.as_reference()?)?.as_stream()?;
            let content = to_unicode.decompressed_content()?;
            let map = parse_to_unicode(&content);
            font_maps.insert(String::from_utf8_lossy(name).to_string(), map);
            println!("Loaded map for font {}", String::from_utf8_lossy(name));
        }
    }

    let contents = page.get(b"Contents")?;
    let content_streams = match contents {
        Object::Reference(id) => vec![doc.get_object(*id)?.as_stream()?],
        Object::Array(arr) => arr.iter().filter_map(|r| doc.get_object(r.as_reference().ok()?).ok()?.as_stream().ok()).collect(),
        _ => vec![],
    };

    for stream in content_streams {
        let content = stream.decompressed_content()?;
        let content_str = String::from_utf8_lossy(&content);
        // Simple parsing of operations
        let mut current_font = String::new();
        let mut current_tm = String::new();

        // Split by 'TJ' or 'Tj' or 'Tf' or 'Tm'
        // This is a very rough parser, just to find the text
        // We'll iterate tokens
        let tokens: Vec<&str> = content_str.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let token = tokens[i];
            if token == "Tf" {
                if i > 1 {
                    current_font = tokens[i-2].trim_start_matches('/').to_string();
                }
            } else if token == "Tm" {
                if i > 5 {
                    current_tm = format!("{} {} {} {} {} {}", tokens[i-6], tokens[i-5], tokens[i-4], tokens[i-3], tokens[i-2], tokens[i-1]);
                }
            } else if token == "Tj" {
                if i > 0 {
                    let text_hex = tokens[i-1];
                    decode_and_print(text_hex, &current_font, &font_maps, &current_tm);
                }
            } else if token == "TJ" {
                // TJ array is harder to parse from tokens because of spaces
                // We'll just look backwards for '['
                // This is hacky but might work for simple cases
                let mut j = i - 1;
                let mut collected = String::new();
                while j > 0 {
                    collected.insert_str(0, tokens[j]);
                    if tokens[j].contains('[') {
                        break;
                    }
                    j -= 1;
                }
                decode_and_print(&collected, &current_font, &font_maps, &current_tm);
            }
            i += 1;
        }
    }

    Ok(())
}

fn parse_to_unicode(content: &[u8]) -> HashMap<u16, String> {
    let s = String::from_utf8_lossy(content);
    let mut map = HashMap::new();
    for line in s.lines() {
        if line.contains("beginbfchar") {
            continue;
        }
        if line.contains("endbfchar") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let code_str = parts[0].trim_matches('<').trim_matches('>');
            let uni_str = parts[1].trim_matches('<').trim_matches('>');
            if let Ok(code) = u16::from_str_radix(code_str, 16) {
                // Handle unicode hex string
                let mut uni = String::new();
                let mut chars = uni_str.chars();
                while let (Some(c1), Some(c2), Some(c3), Some(c4)) = (chars.next(), chars.next(), chars.next(), chars.next()) {
                    let hex: String = vec![c1, c2, c3, c4].into_iter().collect();
                    if let Ok(u) = u16::from_str_radix(&hex, 16) {
                        if let Some(c) = std::char::from_u32(u as u32) {
                            uni.push(c);
                        }
                    }
                }
                map.insert(code, uni);
            }
        }
    }
    map
}

fn decode_and_print(text: &str, font: &str, maps: &HashMap<String, HashMap<u16, String>>, tm: &str) {
    if let Some(map) = maps.get(font) {
        let mut decoded = String::new();
        let mut hex_buffer = String::new();
        let mut in_hex = false;
        
        for c in text.chars() {
            if c == '<' {
                in_hex = true;
                hex_buffer.clear();
            } else if c == '>' {
                in_hex = false;
                // Process hex buffer
                let mut chars = hex_buffer.chars();
                while let (Some(c1), Some(c2), Some(c3), Some(c4)) = (chars.next(), chars.next(), chars.next(), chars.next()) {
                     let hex: String = vec![c1, c2, c3, c4].into_iter().collect();
                     if let Ok(code) = u16::from_str_radix(&hex, 16) {
                         if let Some(u) = map.get(&code) {
                             decoded.push_str(u);
                         } else {
                             decoded.push('?');
                         }
                     }
                }
            } else if in_hex {
                hex_buffer.push(c);
            }
        }
        
        if decoded.contains("Frequency") || decoded.contains("Domain") {
            println!("Found text: '{}' with Font {} at Tm [{}]", decoded, font, tm);
        }
    }
}
