//! WinHTTP 使用 Windows 的证书和 TLS 实现，每次请求均设有超时。
use std::ffi::c_void;
use windows::{
    Win32::Networking::WinHttp::*,
    core::{PCWSTR, w},
};
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
fn handle(p: *mut c_void) -> Result<Handle, String> {
    if p.is_null() {
        Err("WinHTTP 无法创建请求".into())
    } else {
        Ok(Handle(p))
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
// 错误只记录阶段和系统代码，避免 URL 或认证头出现在日志里。
fn fail(stage: &str) -> String {
    format!(
        "{stage}，系统代码 {}",
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    )
}
pub fn post(url: &str, token: &str, body: &[u8]) -> Result<(), String> {
    let (secure, rest) = if let Some(s) = url.strip_prefix("https://") {
        (true, s)
    } else if let Some(s) = url.strip_prefix("http://") {
        (false, s)
    } else {
        return Err("上传地址需要 http:// 或 https://".into());
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        (h, p.parse::<u16>().map_err(|_| "上传端口无效")?)
    } else {
        (authority, if secure { 443 } else { 80 })
    };
    if host.is_empty() || authority.contains('@') || token.contains(['\r', '\n']) {
        return Err("上传配置无效".into());
    }
    let host = wide(host);
    let path = wide(&format!("/{path}"));
    unsafe {
        let session = handle(WinHttpOpen(
            w!("mijia-windows-gateway"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            None,
            None,
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10000, 10000, 30000, 30000)
            .map_err(|_| fail("设置上传超时失败"))?;
        let connection = handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), port, 0))?;
        let request = handle(WinHttpOpenRequest(
            connection.0,
            w!("POST"),
            PCWSTR(path.as_ptr()),
            None,
            None,
            std::ptr::null(),
            if secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            },
        ))?;
        // 禁止跳转，保证认证令牌只发送给配置中的服务器。
        let policy = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&policy.to_ne_bytes()),
        )
        .map_err(|_| fail("设置上传策略失败"))?;
        let headers = wide(&format!(
            "Content-Type: application/json\r\nAuthorization: Bearer {token}\r\n"
        ));
        WinHttpSendRequest(
            request.0,
            Some(&headers[..headers.len() - 1]),
            Some(body.as_ptr().cast()),
            body.len() as u32,
            body.len() as u32,
            0,
        )
        .map_err(|_| fail("发送上传请求失败"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|_| fail("接收上传响应失败"))?;
        let mut status = 0u32;
        let mut size = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            None,
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|_| fail("读取上传状态失败"))?;
        if !(200..300).contains(&status) {
            return Err(format!("服务器返回 HTTP {status}"));
        }
    }
    Ok(())
}
