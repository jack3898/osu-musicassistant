//! A minimal, read-only, dynamically-typed view over a Realm file using realm-core's C API.
//!
//! Only what's needed to read osu!lazer's database is wrapped. Files are always opened in
//! immutable mode, so realm-core never writes to them, never creates lock files and never
//! upgrades their file format. Every handle borrows the [`Realm`] it came from and is released
//! on drop.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::NonNull;

use realm_sys as sys;

#[derive(Debug, thiserror::Error)]
pub enum RealmError {
    #[error("realm error {code}: {message}")]
    Core { code: i32, message: String },
    #[error("class `{0}` not found in the realm file")]
    MissingClass(String),
    #[error("property `{class}.{property}` not found in the realm file")]
    MissingProperty { class: String, property: String },
    #[error("expected {expected} for `{property}`, found {found:?}")]
    UnexpectedType {
        property: String,
        expected: &'static str,
        found: Value,
    },
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

pub type Result<T, E = RealmError> = std::result::Result<T, E>;

/// Take realm-core's thread-local "last error" after a failed call.
fn last_error() -> RealmError {
    let mut err = sys::realm_error_t::default();
    unsafe {
        if sys::realm_get_last_error(&mut err) {
            let message = cstr_to_string(err.message);
            sys::realm_clear_last_error();
            // The enum's C type is unsigned on some compilers, but its values are negative error codes.
            #[allow(clippy::unnecessary_cast)]
            let code = err.error.0 as i32;
            RealmError::Core { code, message }
        } else {
            RealmError::Core {
                code: -1,
                message: "unknown error".into(),
            }
        }
    }
}

fn check(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(last_error()) }
}

fn non_null<T>(ptr: *mut T) -> Result<NonNull<T>> {
    NonNull::new(ptr).ok_or_else(last_error)
}

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Owns a pointer returned by realm-core and releases it on drop.
struct Handle<T>(NonNull<T>);

impl<T> Handle<T> {
    fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }
}

impl<T> Drop for Handle<T> {
    fn drop(&mut self) {
        unsafe { sys::realm_release(self.0.as_ptr().cast()) }
    }
}

/// An open, immutable Realm file. Realm handles are thread-confined, so this is `!Send`.
pub struct Realm {
    handle: Handle<sys::realm_t>,
}

impl Realm {
    /// Open `path` read-only. The file must not be modified while it is open.
    pub fn open_immutable(path: &Path) -> Result<Self> {
        let c_path = CString::new(
            path.to_str()
                .ok_or_else(|| RealmError::InvalidPath(path.display().to_string()))?,
        )
        .map_err(|_| RealmError::InvalidPath(path.display().to_string()))?;
        unsafe {
            let config = Handle(non_null(sys::realm_config_new())?);
            sys::realm_config_set_path(config.as_ptr(), c_path.as_ptr());
            sys::realm_config_set_schema_mode(
                config.as_ptr(),
                sys::realm_schema_mode::RLM_SCHEMA_MODE_IMMUTABLE,
            );
            sys::realm_config_set_cached(config.as_ptr(), false);
            sys::realm_config_set_automatic_change_notifications(config.as_ptr(), false);
            let realm = non_null(sys::realm_open(config.as_ptr()))?;
            Ok(Self {
                handle: Handle(realm),
            })
        }
    }

    fn ptr(&self) -> *const sys::realm_t {
        self.handle.as_ptr()
    }

    /// The application-level schema version (osu!lazer's `RealmAccess.schema_version`).
    pub fn schema_version(&self) -> u64 {
        unsafe { sys::realm_get_schema_version(self.ptr()) }
    }

    /// Names of every class (table) in the file.
    pub fn class_names(&self) -> Result<Vec<String>> {
        unsafe {
            let n = sys::realm_get_num_classes(self.ptr());
            let mut keys = vec![0; n];
            let mut out_n = 0;
            check(sys::realm_get_class_keys(
                self.ptr(),
                keys.as_mut_ptr(),
                n,
                &mut out_n,
            ))?;
            keys.truncate(out_n);
            keys.into_iter()
                .map(|key| {
                    let mut info = sys::realm_class_info_t::default();
                    check(sys::realm_get_class(self.ptr(), key, &mut info))?;
                    Ok(cstr_to_string(info.name))
                })
                .collect()
        }
    }

    /// Look up a class and its properties by its stored (`[MapTo]`) name.
    pub fn class(&self, name: &str) -> Result<Class> {
        let c_name = CString::new(name).map_err(|_| RealmError::MissingClass(name.into()))?;
        unsafe {
            let mut found = false;
            let mut info = sys::realm_class_info_t::default();
            check(sys::realm_find_class(
                self.ptr(),
                c_name.as_ptr(),
                &mut found,
                &mut info,
            ))?;
            if !found {
                return Err(RealmError::MissingClass(name.into()));
            }
            let mut props = vec![sys::realm_property_info_t::default(); info.num_properties];
            let mut out_n = 0;
            check(sys::realm_get_class_properties(
                self.ptr(),
                info.key,
                props.as_mut_ptr(),
                props.len(),
                &mut out_n,
            ))?;
            props.truncate(out_n);
            let properties = props
                .into_iter()
                .map(|p| {
                    let property = Property {
                        key: p.key,
                        is_collection: p.collection_type
                            != sys::realm_collection_type::RLM_COLLECTION_TYPE_NONE,
                    };
                    (cstr_to_string(p.name), property)
                })
                .collect();
            Ok(Class {
                key: info.key,
                name: name.into(),
                properties,
            })
        }
    }

    /// Every object of `class`.
    pub fn all<'r>(&'r self, class: &Class) -> Result<Results<'r>> {
        let results = unsafe { non_null(sys::realm_object_find_all(self.ptr(), class.key))? };
        Ok(Results {
            handle: Handle(results),
            _realm: PhantomData,
        })
    }

    /// Resolve a link value to the object it points at.
    pub fn object<'r>(&'r self, link: Link) -> Result<Object<'r>> {
        let obj = unsafe { non_null(sys::realm_get_object(self.ptr(), link.class, link.key))? };
        Ok(Object {
            handle: Handle(obj),
            _realm: PhantomData,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Property {
    key: sys::realm_property_key_t,
    is_collection: bool,
}

#[derive(Debug, Clone)]
pub struct Class {
    key: sys::realm_class_key_t,
    name: String,
    properties: HashMap<String, Property>,
}

impl Class {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn has_property(&self, name: &str) -> bool {
        self.properties.contains_key(name)
    }

    /// Look up a property; fails if lazer's schema no longer has it.
    pub fn property(&self, name: &str) -> Result<Property> {
        self.properties
            .get(name)
            .copied()
            .ok_or_else(|| RealmError::MissingProperty {
                class: self.name.clone(),
                property: name.into(),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Link {
    class: sys::realm_class_key_t,
    key: sys::realm_object_key_t,
}

/// A single property value, copied out of the Realm.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Bool(bool),
    String(String),
    Float(f32),
    Double(f64),
    Uuid([u8; 16]),
    /// Seconds and nanoseconds since the Unix epoch.
    Timestamp {
        seconds: i64,
        nanos: i32,
    },
    Link(Link),
    /// A type we don't read (binary, decimal128, object id, ...).
    Unsupported(u32),
}

impl Value {
    unsafe fn from_raw(v: &sys::realm_value_t) -> Self {
        use sys::realm_value_type as T;
        unsafe {
            let u = &v.__bindgen_anon_1;
            match v.type_ {
                T::RLM_TYPE_NULL => Value::Null,
                T::RLM_TYPE_INT => Value::Int(u.integer),
                T::RLM_TYPE_BOOL => Value::Bool(u.boolean),
                T::RLM_TYPE_STRING => {
                    let s = u.string;
                    if s.data.is_null() {
                        Value::String(String::new())
                    } else {
                        let bytes = std::slice::from_raw_parts(s.data.cast::<u8>(), s.size);
                        Value::String(String::from_utf8_lossy(bytes).into_owned())
                    }
                }
                T::RLM_TYPE_FLOAT => Value::Float(u.fnum),
                T::RLM_TYPE_DOUBLE => Value::Double(u.dnum),
                T::RLM_TYPE_UUID => Value::Uuid(u.uuid.bytes),
                T::RLM_TYPE_TIMESTAMP => Value::Timestamp {
                    seconds: u.timestamp.seconds,
                    nanos: u.timestamp.nanoseconds,
                },
                T::RLM_TYPE_LINK => Value::Link(Link {
                    class: u.link.target_table,
                    key: u.link.target,
                }),
                other => Value::Unsupported(other.0),
            }
        }
    }
}

pub struct Results<'r> {
    handle: Handle<sys::realm_results_t>,
    _realm: PhantomData<&'r Realm>,
}

impl<'r> Results<'r> {
    pub fn len(&self) -> Result<usize> {
        let mut n = 0;
        check(unsafe { sys::realm_results_count(self.handle.as_ptr(), &mut n) })?;
        Ok(n)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn get(&self, index: usize) -> Result<Object<'r>> {
        let obj = unsafe { non_null(sys::realm_results_get_object(self.handle.as_ptr(), index))? };
        Ok(Object {
            handle: Handle(obj),
            _realm: PhantomData,
        })
    }

    pub fn iter(&self) -> Result<impl Iterator<Item = Result<Object<'r>>> + '_> {
        Ok((0..self.len()?).map(|i| self.get(i)))
    }
}

pub struct Object<'r> {
    handle: Handle<sys::realm_object_t>,
    _realm: PhantomData<&'r Realm>,
}

impl<'r> Object<'r> {
    pub fn get(&self, property: Property) -> Result<Value> {
        let mut raw = sys::realm_value_t::default();
        check(unsafe { sys::realm_get_value(self.handle.as_ptr(), property.key, &mut raw) })?;
        Ok(unsafe { Value::from_raw(&raw) })
    }

    pub fn list(&self, property: Property) -> Result<List<'r>> {
        debug_assert!(property.is_collection);
        let list = unsafe { non_null(sys::realm_get_list(self.handle.as_ptr(), property.key))? };
        Ok(List {
            handle: Handle(list),
            _realm: PhantomData,
        })
    }

    /// Follow a to-one link property. `None` if the link is null.
    pub fn linked(&self, property: Property) -> Result<Option<Object<'r>>> {
        if self.get(property)? == Value::Null {
            return Ok(None);
        }
        let obj = unsafe {
            non_null(sys::realm_get_linked_object(
                self.handle.as_ptr(),
                property.key,
            ))?
        };
        Ok(Some(Object {
            handle: Handle(obj),
            _realm: PhantomData,
        }))
    }
}

pub struct List<'r> {
    handle: Handle<sys::realm_list_t>,
    _realm: PhantomData<&'r Realm>,
}

impl<'r> List<'r> {
    pub fn len(&self) -> Result<usize> {
        let mut n = 0;
        check(unsafe { sys::realm_list_size(self.handle.as_ptr(), &mut n) })?;
        Ok(n)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn get(&self, index: usize) -> Result<Value> {
        let mut raw = sys::realm_value_t::default();
        check(unsafe { sys::realm_list_get(self.handle.as_ptr(), index, &mut raw) })?;
        Ok(unsafe { Value::from_raw(&raw) })
    }

    /// The linked (or embedded) object at `index`.
    pub fn object(&self, index: usize) -> Result<Object<'r>> {
        let obj = unsafe {
            non_null(sys::realm_list_get_linked_object(
                self.handle.as_ptr(),
                index,
            ))?
        };
        Ok(Object {
            handle: Handle(obj),
            _realm: PhantomData,
        })
    }
}
