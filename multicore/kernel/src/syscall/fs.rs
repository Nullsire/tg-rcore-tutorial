use crate::mm::{translated_byte_buffer, translated_refmut};
use crate::task::processor::{current_task, current_user_token};
use alloc::string::String;

fn translated_string(token: usize, ptr: *const u8, len: usize) -> Option<String> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let mut bytes = alloc::vec::Vec::with_capacity(len);
    for chunk in translated_byte_buffer(token, ptr, len) {
        bytes.extend_from_slice(chunk);
    }
    core::str::from_utf8(&bytes).ok().map(String::from)
}

pub fn sys_open(path: *const u8, len: usize, flags: usize) -> isize {
    let token = current_user_token();
    let Some(path) = translated_string(token, path, len) else {
        return -1;
    };

    let Some(file) = crate::fs::RamFile::open(path.as_str(), flags) else {
        return -1;
    };

    let task = current_task().unwrap();
    let process = task.get_process();
    let mut inner = process.inner.lock();

    // Reserve 0/1/2 for stdio. Reuse holes from 3 onward.
    while inner.fd_table.len() < 3 {
        inner.fd_table.push(None);
    }
    for fd in 3..inner.fd_table.len() {
        if inner.fd_table[fd].is_none() {
            inner.fd_table[fd] = Some(file.clone());
            return fd as isize;
        }
    }
    inner.fd_table.push(Some(file));
    (inner.fd_table.len() - 1) as isize
}

pub fn sys_close(fd: usize) -> isize {
    if fd < 3 {
        return -1;
    }
    let task = current_task().unwrap();
    let process = task.get_process();
    let mut inner = process.inner.lock();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd] = None;
    0
}

pub fn sys_lseek(fd: usize, offset: isize, whence: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();
    let inner = process.inner.lock();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    let Some(file) = &inner.fd_table[fd] else {
        return -1;
    };
    file.seek(offset, whence)
}

pub fn sys_unlink(path: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let Some(path) = translated_string(token, path, len) else {
        return -1;
    };
    crate::fs::ramfs::unlink(path.as_str())
}

pub fn sys_fsize(fd: usize, size_ptr: *mut usize) -> isize {
    if size_ptr.is_null() {
        return -1;
    }
    let task = current_task().unwrap();
    let process = task.get_process();
    let inner = process.inner.lock();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    let Some(file) = &inner.fd_table[fd] else {
        return -1;
    };
    let size = file.size();
    drop(inner);
    let token = current_user_token();
    *translated_refmut(token, size_ptr) = size;
    0
}

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let task = current_task().unwrap();
    let process = task.get_process();
    let inner = process.inner.lock();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        drop(inner);
        let buffers = translated_byte_buffer(token, buf, len);
        let mut total = 0;
        for buffer in buffers {
            total += file.write(buffer);
        }
        total as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let task = current_task().unwrap();
    let process = task.get_process();
    let inner = process.inner.lock();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.readable() {
            return -1;
        }
        let file = file.clone();
        drop(inner);
        let buffers = translated_byte_buffer(token, buf, len);
        let mut total = 0;
        for buffer in buffers {
            total += file.read(buffer);
        }
        total as isize
    } else {
        -1
    }
}
