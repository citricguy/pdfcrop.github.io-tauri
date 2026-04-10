use anyhow::Result;
use clap::Parser;
use lopdf::Document;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    input: PathBuf,
    #[arg(long)]
    page: Option<u32>,
    #[arg(long)]
    object: Option<u32>,
    #[arg(long)]
    scan_forms: bool,
    #[arg(long)]
    find_xobject: Option<u32>,
    #[arg(long)]
    find_ref: Option<u32>,
    #[arg(long)]
    find_resource: Option<u32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let doc = Document::load(&args.input)?;
    
    if let Some(target_id) = args.find_resource {
        println!("Scanning all resource dictionaries for mapping to {}...", target_id);
        for (id, object) in doc.objects.iter() {
            let dict = if let Ok(d) = object.as_dict() {
                Some(d)
            } else if let Ok(s) = object.as_stream() {
                Some(&s.dict)
            } else {
                None
            };

            if let Some(dict) = dict {
                if let Ok(resources) = dict.get(b"Resources") {
                     let resources_dict = match resources {
                        lopdf::Object::Reference(rid) => doc.get_object(*rid).ok().and_then(|o| o.as_dict().ok()),
                        lopdf::Object::Dictionary(d) => Some(d),
                        _ => None,
                    };
                    
                    if let Some(res) = resources_dict {
                        // Check XObject sub-dictionary
                        if let Ok(xobjects) = res.get(b"XObject") {
                             let xobjects_dict = match xobjects {
                                lopdf::Object::Reference(xid) => doc.get_object(*xid).ok().and_then(|o| o.as_dict().ok()),
                                lopdf::Object::Dictionary(d) => Some(d),
                                _ => None,
                            };
                            if let Some(xobj_map) = xobjects_dict {
                                for (name, ref_obj) in xobj_map.iter() {
                                    if let Ok(rid) = ref_obj.as_reference() {
                                        if rid.0 == target_id {
                                            println!("Object {:?} maps name {} to {}", id, String::from_utf8_lossy(name), target_id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(target_id) = args.find_ref {
        println!("Scanning all objects for reference to {}...", target_id);
        for (id, object) in doc.objects.iter() {
            // Check dictionary values
            if let Ok(dict) = object.as_dict() {
                for (key, value) in dict.iter() {
                    if let Ok(ref_id) = value.as_reference() {
                        if ref_id.0 == target_id {
                            println!("Object {:?} references {} in key {}", id, target_id, String::from_utf8_lossy(key));
                        }
                    }
                    // Check arrays in dict
                    if let Ok(arr) = value.as_array() {
                        for (i, item) in arr.iter().enumerate() {
                            if let Ok(ref_id) = item.as_reference() {
                                if ref_id.0 == target_id {
                                    println!("Object {:?} references {} in key {} at index {}", id, target_id, String::from_utf8_lossy(key), i);
                                }
                            }
                        }
                    }
                }
            }
            // Check array values
            if let Ok(arr) = object.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    if let Ok(ref_id) = item.as_reference() {
                        if ref_id.0 == target_id {
                            println!("Object {:?} references {} at index {}", id, target_id, i);
                        }
                    }
                }
            }
            // Check stream dictionary
            if let Ok(stream) = object.as_stream() {
                for (key, value) in stream.dict.iter() {
                    if let Ok(ref_id) = value.as_reference() {
                        if ref_id.0 == target_id {
                            println!("Object {:?} (Stream) references {} in key {}", id, target_id, String::from_utf8_lossy(key));
                        }
                    }
                     // Check arrays in dict
                    if let Ok(arr) = value.as_array() {
                        for (i, item) in arr.iter().enumerate() {
                            if let Ok(ref_id) = item.as_reference() {
                                if ref_id.0 == target_id {
                                    println!("Object {:?} (Stream) references {} in key {} at index {}", id, target_id, String::from_utf8_lossy(key), i);
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(target_id) = args.find_xobject {
        println!("Scanning pages for reference to XObject {}...", target_id);
        for (page_num, page_id) in doc.get_pages() {
            if let Ok(page) = doc.get_object(page_id) {
                if let Ok(dict) = page.as_dict() {
                    if let Ok(resources_obj) = dict.get(b"Resources") {
                        let resources_dict = match resources_obj {
                            lopdf::Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
                            lopdf::Object::Dictionary(d) => Some(d),
                            _ => None,
                        };
                        
                        if let Some(resources) = resources_dict {
                            if let Ok(xobjects) = resources.get(b"XObject") {
                                let xobjects_dict = match xobjects {
                                    lopdf::Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
                                    lopdf::Object::Dictionary(d) => Some(d),
                                    _ => None,
                                };

                                if let Some(xobjects) = xobjects_dict {
                                    for (name, obj) in xobjects.iter() {
                                        if let Ok(oid) = obj.as_reference() {
                                            if oid.0 == target_id {
                                                println!("Found XObject {} on Page {} (ID {:?}) as name {}", target_id, page_num, page_id, String::from_utf8_lossy(name));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    if args.scan_forms {
        println!("Scanning for Form XObjects...");
        for (id, object) in doc.objects.iter() {
            if let Ok(stream) = object.as_stream() {
                if let Ok(subtype) = stream.dict.get(b"Subtype") {
                    if let Ok(name) = subtype.as_name() {
                        if name == b"Form" {
                            println!("Found Form XObject: {:?}", id);
                            if let Ok(content) = stream.decompressed_content() {
                                let decoded = String::from_utf8_lossy(&content);
                                println!("Content: {}", decoded);
                                println!("---");
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(obj_id) = args.object {
        let object = doc.get_object((obj_id, 0))?;
        if let Ok(stream) = object.as_stream() {
            println!("Dictionary: {:?}", stream.dict);
            let content = stream.decompressed_content()?;
            let decoded = String::from_utf8_lossy(&content);
            println!("{}", decoded);
        } else {
            println!("Object {} is not a stream", obj_id);
            println!("{:?}", object);
        }
        return Ok(());
    }

    if let Some(page_num) = args.page {
        let page_id = *doc.get_pages().get(&page_num).ok_or_else(|| anyhow::anyhow!("Page not found"))?;
        println!("Page ID: {:?}", page_id);
        let content = doc.get_page_content(page_id)?;
        let decoded = String::from_utf8_lossy(&content);
        
        println!("{}", decoded);
        
        if let Ok(page) = doc.get_object(page_id) {
            if let Ok(dict) = page.as_dict() {
                if let Ok(resources_obj) = dict.get(b"Resources") {
                    let resources_dict = match resources_obj {
                        lopdf::Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
                        lopdf::Object::Dictionary(d) => Some(d),
                        _ => None,
                    };
                    
                    if let Some(resources) = resources_dict {
                        if let Ok(xobjects) = resources.get(b"XObject") {
                            let xobjects_dict = match xobjects {
                                lopdf::Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
                                lopdf::Object::Dictionary(d) => Some(d),
                                _ => None,
                            };

                            if let Some(xobjects) = xobjects_dict {
                                println!("\n--- XObjects ---");
                                for (name, obj) in xobjects.iter() {
                                    println!("XObject: {} -> {:?}", String::from_utf8_lossy(name), obj);
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        println!("Please specify --page or --object");
    }

    Ok(())
}
