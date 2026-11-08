//! 待上传队列：直接使用 Windows 自带的 winsqlite3.dll，不额外打包 SQLite。
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
    ptr,
};

#[link(name = "winsqlite3", kind = "raw-dylib")]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, db: *mut *mut c_void) -> c_int;
    fn sqlite3_close(db: *mut c_void) -> c_int;
    fn sqlite3_busy_timeout(db: *mut c_void, ms: c_int) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut c_void,
        sql: *const c_char,
        n: c_int,
        stmt: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    fn sqlite3_step(stmt: *mut c_void) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut c_void,
        i: c_int,
        text: *const c_char,
        n: c_int,
        destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_column_text(stmt: *mut c_void, i: c_int) -> *const u8;
    fn sqlite3_column_int64(stmt: *mut c_void, i: c_int) -> i64;
    fn sqlite3_column_double(stmt: *mut c_void, i: c_int) -> f64;
    fn sqlite3_column_type(stmt: *mut c_void, i: c_int) -> c_int;
}
pub struct Store(*mut c_void);
struct Statement(*mut c_void);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.0);
        }
    }
}
impl Drop for Statement {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.0);
        }
    }
}
type Result<T> = std::result::Result<T, String>;
fn check(code: c_int) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("SQLite 操作失败，代码 {code}"))
    }
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "无法创建数据库目录")?;
        }
        let name = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "数据库路径无效")?;
        let mut db = ptr::null_mut();
        let code = unsafe { sqlite3_open(name.as_ptr(), &mut db) };
        let store = Self(db);
        check(code)?;
        unsafe {
            sqlite3_busy_timeout(db, 10000);
        }
        for sql in [
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=FULL",
            "CREATE TABLE IF NOT EXISTS readings (id INTEGER PRIMARY KEY,address TEXT NOT NULL,ts INTEGER NOT NULL,temperature REAL NOT NULL,humidity INTEGER NOT NULL,voltage REAL,battery INTEGER)",
            "CREATE UNIQUE INDEX IF NOT EXISTS readings_address_ts_unique ON readings(address,ts)",
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY,value TEXT)",
            "CREATE TABLE IF NOT EXISTS devices (address TEXT PRIMARY KEY,model TEXT,firmware TEXT,hardware TEXT,updated_ts INTEGER)",
        ] {
            store.exec(sql, &[])?;
        }
        Ok(store)
    }
    fn prepare(&self, sql: &str, args: &[String]) -> Result<Statement> {
        let sql = CString::new(sql).map_err(|_| "SQL 格式错误")?;
        let mut stmt = ptr::null_mut();
        check(unsafe { sqlite3_prepare_v2(self.0, sql.as_ptr(), -1, &mut stmt, ptr::null_mut()) })?;
        let stmt = Statement(stmt);
        for (i, arg) in args.iter().enumerate() {
            // SQLITE_TRANSIENT：SQLite 在调用返回前复制参数。
            let transient = unsafe {
                std::mem::transmute::<isize, Option<unsafe extern "C" fn(*mut c_void)>>(-1)
            };
            check(unsafe {
                sqlite3_bind_text(
                    stmt.0,
                    i as i32 + 1,
                    arg.as_ptr().cast(),
                    arg.len() as i32,
                    transient,
                )
            })?;
        }
        Ok(stmt)
    }
    fn exec(&self, sql: &str, args: &[String]) -> Result<()> {
        let stmt = self.prepare(sql, args)?;
        loop {
            match unsafe { sqlite3_step(stmt.0) } {
                100 => (),
                101 => return Ok(()),
                n => return Err(format!("SQLite 写入失败，代码 {n}")),
            }
        }
    }
    fn text(stmt: &Statement, i: i32) -> Option<String> {
        unsafe {
            let p = sqlite3_column_text(stmt.0, i);
            if p.is_null() {
                None
            } else {
                Some(CStr::from_ptr(p.cast()).to_string_lossy().into_owned())
            }
        }
    }
    pub fn restore(&self, address: &str) -> Result<Option<Value>> {
        let stmt = self.prepare(
            "SELECT value FROM meta WHERE key=?",
            &[format!("rust_device:{address}")],
        )?;
        if unsafe { sqlite3_step(stmt.0) } == 100 {
            return Ok(Self::text(&stmt, 0).and_then(|s| serde_json::from_str(&s).ok()));
        }
        // 没有保存过状态时，用队列里最新的读数和电量恢复。
        let stmt = self.prepare(
            "SELECT ts,temperature,humidity FROM readings WHERE address=? ORDER BY ts DESC LIMIT 1",
            &[address.into()],
        )?;
        let mut state = json!({});
        if unsafe { sqlite3_step(stmt.0) } == 100 {
            unsafe {
                state["last_ok"] = json!(sqlite3_column_int64(stmt.0, 0));
                state["last_saved"] = state["last_ok"].clone();
                state["temperature"] = json!(sqlite3_column_double(stmt.0, 1));
                state["humidity"] = json!(sqlite3_column_int64(stmt.0, 2));
            }
        }
        let stmt=self.prepare("SELECT ts,battery FROM readings WHERE address=? AND battery IS NOT NULL ORDER BY ts DESC LIMIT 1",&[address.into()])?;
        if unsafe { sqlite3_step(stmt.0) } == 100 {
            unsafe {
                state["battery_ts"] = json!(sqlite3_column_int64(stmt.0, 0));
                state["battery"] = json!(sqlite3_column_int64(stmt.0, 1));
            }
        }
        let stmt = self.prepare(
            "SELECT model,firmware FROM devices WHERE address=?",
            &[address.into()],
        )?;
        if unsafe { sqlite3_step(stmt.0) } == 100 {
            state["model"] = json!(Self::text(&stmt, 0));
            state["firmware"] = json!(Self::text(&stmt, 1));
        }
        Ok(Some(state))
    }
    pub fn persist(&self, address: &str, state: &Value, reading: Option<&Value>) -> Result<()> {
        self.exec("BEGIN IMMEDIATE", &[])?;
        let result = (|| {
            if let Some(r) = reading {
                self.exec("INSERT OR IGNORE INTO readings(address,ts,temperature,humidity,voltage,battery) VALUES(?,?,?,?,NULL,CAST(NULLIF(?, '') AS INTEGER))",&[
                    address.into(),r["ts"].to_string(),r["temperature"].to_string(),r["humidity"].to_string(),r["battery"].as_i64().map(|n|n.to_string()).unwrap_or_default()])?;
            }
            self.exec("INSERT INTO meta(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",&[format!("rust_device:{address}"),state.to_string()])?;
            self.exec("COMMIT", &[])
        })();
        if result.is_err() {
            let _ = self.exec("ROLLBACK", &[]);
        }
        result
    }
    pub fn pending(&self) -> Result<Vec<Value>> {
        let stmt=self.prepare("SELECT id,address,ts,temperature,humidity,voltage,battery FROM readings ORDER BY id LIMIT 1000",&[])?;
        let mut rows = Vec::new();
        loop {
            unsafe {
                match sqlite3_step(stmt.0) {
            100=>rows.push(json!({"id":sqlite3_column_int64(stmt.0,0),"address":Self::text(&stmt,1),"ts":sqlite3_column_int64(stmt.0,2),"temperature":sqlite3_column_double(stmt.0,3),"humidity":sqlite3_column_int64(stmt.0,4),"voltage": if sqlite3_column_type(stmt.0,5)==5 {None}else{Some(sqlite3_column_double(stmt.0,5))},"battery":if sqlite3_column_type(stmt.0,6)==5 {None}else{Some(sqlite3_column_int64(stmt.0,6))}})),
            101=>return Ok(rows),n=>return Err(format!("SQLite 查询失败，代码 {n}")),
        }
            }
        }
    }
    pub fn delete_upto(&self, id: i64) -> Result<()> {
        self.exec("DELETE FROM readings WHERE id<=?", &[id.to_string()])
    }
}
