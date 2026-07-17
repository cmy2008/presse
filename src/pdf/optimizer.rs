use std::collections::BTreeMap;
use std::hash::{Hash, Hasher, DefaultHasher};
use lopdf::{Document, Object, ObjectId};

const MIN_STREAM_SIZE: usize = 256;

pub fn optimize(doc: &mut Document) {
    deduplicate_font_programs(doc);
}

fn deduplicate_font_programs(doc: &mut Document) {
    let mut font_programs: BTreeMap<ObjectId, u64> = BTreeMap::new();

    let all_ids: Vec<ObjectId> = doc.objects.keys().copied().collect();

    for &id in &all_ids {
        let font_file_refs = get_font_file_refs(doc, id);
        for ff_id in font_file_refs {
            if font_programs.contains_key(&ff_id) {
                continue;
            }
            if let Some(Object::Stream(stream)) = doc.objects.get(&ff_id) {
                if stream.content.len() < MIN_STREAM_SIZE {
                    continue;
                }
                let hash = stream.decompressed_content()
                    .ok()
                    .map(|data| {
                        let mut h = DefaultHasher::new();
                        data.hash(&mut h);
                        h.finish()
                    })
                    .unwrap_or_else(|| {
                        let mut h = DefaultHasher::new();
                        stream.content.hash(&mut h);
                        h.finish()
                    });
                font_programs.insert(ff_id, hash);
            }
        }
    }

    let mut hash_to_canonical: BTreeMap<u64, ObjectId> = BTreeMap::new();
    let mut redirects: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();

    for (&id, &hash) in &font_programs {
        match hash_to_canonical.entry(hash) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(id);
            }
            std::collections::btree_map::Entry::Occupied(e) => {
                if *e.get() != id {
                    redirects.insert(id, *e.get());
                }
            }
        }
    }

    if redirects.is_empty() {
        return;
    }

    replace_all_references(doc, &redirects);

    for old_id in redirects.keys() {
        doc.objects.remove(old_id);
    }
}

fn get_font_file_refs(doc: &Document, obj_id: ObjectId) -> Vec<ObjectId> {
    let obj = match doc.objects.get(&obj_id) {
        Some(o) => o,
        None => return Vec::new(),
    };

    let dict = match obj.as_dict() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let is_font = dict.get(b"Type").ok()
        .and_then(|o| o.as_name().ok())
        .map_or(false, |name| name == b"Font");

    if !is_font {
        return Vec::new();
    }

    let desc_ref = match dict.get(b"FontDescriptor").ok()
        .and_then(|o| o.as_reference().ok()) {
        Some(r) => r,
        None => return Vec::new(),
    };

    let desc_obj = match doc.objects.get(&desc_ref) {
        Some(o) => o,
        None => return Vec::new(),
    };

    let desc_dict = match desc_obj.as_dict() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut refs = Vec::new();
    for key in &[b"FontFile" as &[u8], b"FontFile2", b"FontFile3"] {
        if let Some(ff_ref) = desc_dict.get(key).ok()
            .and_then(|o| o.as_reference().ok()) {
            refs.push(ff_ref);
        }
    }
    refs
}

fn replace_all_references(doc: &mut Document, redirects: &BTreeMap<ObjectId, ObjectId>) {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        if let Some(obj) = doc.objects.get_mut(&id) {
            replace_in_obj(obj, redirects);
        }
    }
}

fn replace_in_obj(obj: &mut Object, redirects: &BTreeMap<ObjectId, ObjectId>) {
    match obj {
        Object::Reference(ref mut id) => {
            if let Some(&new_id) = redirects.get(id) {
                *id = new_id;
            }
        }
        Object::Array(ref mut arr) => {
            for item in arr.iter_mut() {
                replace_in_obj(item, redirects);
            }
        }
        Object::Dictionary(ref mut dict) => {
            let keys: Vec<Vec<u8>> = dict.iter().map(|(k, _)| Vec::from(k)).collect();
            for key in &keys {
                if let Ok(val) = dict.get_mut(key) {
                    replace_in_obj(val, redirects);
                }
            }
        }
        Object::Stream(ref mut stream) => {
            let keys: Vec<Vec<u8>> = stream.dict.iter().map(|(k, _)| Vec::from(k)).collect();
            for key in &keys {
                if let Ok(val) = stream.dict.get_mut(key) {
                    replace_in_obj(val, redirects);
                }
            }
        }
        _ => {}
    }
}
