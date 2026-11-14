use std::collections::HashSet;

pub const DEFAULT_EXTENSIONS: &str = "doc,docx,docm,dot,dotx,dotm,wps,wpt,ppt,pptx,pptm,pps,ppsx,ppsm,pot,potx,potm,dps,dpt,xls,xlsx,xlsm,xlsb,xlt,xltx,xltm,et,ett,jpg,jpeg,png,bmp,gif,tif,tiff,webp,ico,heic,heif";

/// Lexical normalization only: never opens a document or follows a reparse point.
pub fn normalize(path: &str) -> String {
    let p = path.replace('/', "\\");
    let p = if let Some(rest) = p.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = p
        .strip_prefix("\\\\?\\")
        .or_else(|| p.strip_prefix("\\??\\"))
    {
        rest.to_owned()
    } else {
        p
    };
    p.trim_end_matches('\\').to_owned()
}

pub fn under(path: &str, root: &str) -> bool {
    let p = normalize(path).to_lowercase();
    let r = normalize(root).to_lowercase();
    !r.is_empty()
        && (p == r
            || p.strip_prefix(&r)
                .is_some_and(|rest| rest.starts_with('\\')))
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub roots: Vec<String>,
    pub excluded: Vec<String>,
    pub extensions: HashSet<String>,
    pub device_maps: Vec<(String, String)>,
}

impl Scope {
    pub fn new(roots: Vec<String>, excluded: Vec<String>, extensions: &str) -> Self {
        let mut result = Self {
            roots: Vec::new(),
            excluded,
            extensions: extensions
                .split(',')
                .map(|s| s.trim().trim_start_matches('.').to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
            device_maps: Vec::new(),
        };
        for root in roots {
            result.add_root(root);
        }
        result
    }

    pub fn add_root(&mut self, root: String) {
        let root = normalize(&root);
        if !self.roots.iter().any(|r| r.eq_ignore_ascii_case(&root)) {
            self.roots.push(root);
        }
    }

    pub fn add_device(&mut self, device: String, drive: String) {
        self.device_maps
            .retain(|(d, _)| !d.eq_ignore_ascii_case(&device));
        self.device_maps.push((device, normalize(&drive)));
        self.device_maps
            .sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
    }

    pub fn translate(&self, path: &str) -> String {
        let p = normalize(path);
        for (device, drive) in &self.device_maps {
            if under(&p, device) {
                return format!("{}{}", drive, &p[device.len()..]);
            }
        }
        if let Some(rest) = p.strip_prefix("\\Device\\Mup\\") {
            return format!("\\\\{rest}");
        }
        p
    }

    pub fn matches(&self, path: &str) -> bool {
        if self.excluded.iter().any(|r| under(path, r)) {
            return false;
        }
        if !self.roots.iter().any(|r| under(path, r)) {
            return false;
        }
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        name.rsplit_once('.')
            .is_some_and(|(_, ext)| self.extensions.contains(&ext.to_lowercase()))
    }

    pub fn extension_list(&self) -> Vec<String> {
        let mut list: Vec<_> = self.extensions.iter().cloned().collect();
        list.sort();
        list
    }
}

/// File object and file key are distinct namespaces. None means a known out-of-scope path.
#[derive(Default)]
pub struct NameCache {
    objects: std::collections::HashMap<u64, Option<String>>,
    keys: std::collections::HashMap<u64, Option<String>>,
}

impl NameCache {
    const LIMIT: usize = 65_536;
    pub fn insert(&mut self, object: Option<u64>, key: Option<u64>, path: Option<String>) -> bool {
        let reset = self.objects.len() + self.keys.len() >= Self::LIMIT;
        if reset {
            self.objects.clear();
            self.keys.clear();
        }
        if let Some(id) = object.filter(|id| *id != 0) {
            self.objects.insert(id, path.clone());
        }
        if let Some(id) = key.filter(|id| *id != 0) {
            self.keys.insert(id, path);
        }
        reset
    }
    pub fn lookup(&self, object: Option<u64>, key: Option<u64>) -> Option<&Option<String>> {
        // FileKey name events are the authoritative mapping when available.
        key.and_then(|id| self.keys.get(&id))
            .or_else(|| object.and_then(|id| self.objects.get(&id)))
    }
    pub fn remove_object(&mut self, object: Option<u64>) {
        if let Some(id) = object {
            self.objects.remove(&id);
        }
    }
    pub fn remove_key(&mut self, key: Option<u64>) {
        if let Some(id) = key {
            self.keys.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope::new(
            vec!["D:\\".into(), "C:\\Users\\测试\\Desktop".into()],
            vec!["D:\\logs".into()],
            DEFAULT_EXTENSIONS,
        )
    }
    #[test]
    fn component_boundaries() {
        assert!(under("D:\\foo\\a.docx", "D:\\foo"));
        assert!(!under("D:\\foobar\\a.docx", "D:\\foo"));
    }
    #[test]
    fn drive_and_case() {
        assert!(scope().matches("d:/lesson/A.PPTX"));
        assert!(!scope().matches("E:\\A.docx"));
    }
    #[test]
    fn excludes_outputs_but_not_neighbors() {
        assert!(!scope().matches("D:\\logs\\a.docx"));
        assert!(scope().matches("D:\\logs-old\\a.docx"));
    }
    #[test]
    fn families_only() {
        assert!(!scope().matches("D:\\a.pdf"));
        assert!(!scope().matches("D:\\a.mp4"));
        assert!(scope().matches("D:\\a.dps"));
        assert!(scope().matches("D:\\a.JPEG"));
    }
    #[test]
    fn unicode_and_extended_paths() {
        assert!(scope().matches("\\\\?\\C:\\Users\\测试\\Desktop\\课件.docx"));
    }
    #[test]
    fn device_longest_prefix() {
        let mut s = scope();
        s.add_device("\\Device\\HarddiskVolume1".into(), "C:".into());
        s.add_device("\\Device\\HarddiskVolume10".into(), "D:".into());
        assert_eq!(
            s.translate("\\Device\\HarddiskVolume10\\a.docx"),
            "D:\\a.docx"
        );
    }
    #[test]
    fn objects_and_keys_do_not_collide() {
        let mut c = NameCache::default();
        c.insert(Some(1), Some(2), Some("a".into()));
        c.insert(Some(2), Some(3), Some("b".into()));
        assert_eq!(c.lookup(Some(1), Some(2)), Some(&Some("a".into())));
        c.remove_object(Some(1));
        assert_eq!(c.lookup(None, Some(2)), Some(&Some("a".into())));
    }
    #[test]
    fn out_of_scope_is_not_unknown() {
        let mut c = NameCache::default();
        c.insert(Some(1), None, None);
        assert_eq!(c.lookup(Some(1), None), Some(&None));
        assert_eq!(c.lookup(Some(9), None), None);
    }
}
