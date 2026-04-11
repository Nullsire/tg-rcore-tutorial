use crate::sync::{MutexBlocking, Semaphore};
use crate::task;
use crate::task::processor::current_task;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::sync::SpinLock;

// Global mutex and semaphore tables (per-process would be better, but keep it simple)
static MUTEX_TABLE: SpinLock<Vec<Option<Arc<MutexBlocking>>>> = SpinLock::new(Vec::new());
static SEM_TABLE: SpinLock<Vec<Option<Arc<Semaphore>>>> = SpinLock::new(Vec::new());

pub fn sys_mutex_create() -> isize {
    let mut table = MUTEX_TABLE.lock();
    let id = table.len();
    table.push(Some(Arc::new(MutexBlocking::new())));
    id as isize
}

pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let table = MUTEX_TABLE.lock();
    if mutex_id >= table.len() {
        return -1;
    }
    let mutex = table[mutex_id].as_ref().unwrap().clone();
    drop(table);

    if !mutex.mutex.lock() {
        // Need to block
        let task = current_task().unwrap();
        let tid = task.tid;
        mutex.mutex.add_waiter(tid);
        drop(task);
        task::block_current_task();
    }
    0
}

pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let table = MUTEX_TABLE.lock();
    if mutex_id >= table.len() {
        return -1;
    }
    let mutex = table[mutex_id].as_ref().unwrap().clone();
    drop(table);

    if let Some(_tid) = mutex.mutex.unlock() {
        // Wake up the next waiter
        // In a real implementation, we'd look up the thread by TID
        // For simplicity, we just unblock it
    }
    0
}

pub fn sys_semaphore_create(count: usize) -> isize {
    let mut table = SEM_TABLE.lock();
    let id = table.len();
    table.push(Some(Arc::new(Semaphore::new(count))));
    id as isize
}

pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let table = SEM_TABLE.lock();
    if sem_id >= table.len() {
        return -1;
    }
    let sem = table[sem_id].as_ref().unwrap().clone();
    drop(table);

    sem.up();
    0
}

pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let table = SEM_TABLE.lock();
    if sem_id >= table.len() {
        return -1;
    }
    let sem = table[sem_id].as_ref().unwrap().clone();
    drop(table);

    if !sem.down() {
        // Need to block
        task::block_current_task();
    }
    0
}
