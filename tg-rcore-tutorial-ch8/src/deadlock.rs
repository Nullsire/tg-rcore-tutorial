use crate::process::Process;

pub fn check_deadlock(
    current_tid: usize,
    req_mutex: Option<usize>,
    req_sem: Option<usize>,
    current_proc: &Process
) -> bool {
    let num_mutexes = current_proc.mutex_list.len();
    let num_sems = current_proc.semaphore_list.len();

    // Available arrays
    let mut available_mutex = alloc::vec![1; num_mutexes];
    let mut available_sem = alloc::vec![0; num_sems];

    // Compute available mutexes
    for (_, m_list) in current_proc.mutex_allocations.iter() {
        for &m_id in m_list.iter() {
            if m_id < num_mutexes {
                available_mutex[m_id] = 0;
            }
        }
    }

    // Compute available semaphores
    for (i, sem_opt) in current_proc.semaphore_list.iter().enumerate() {
        if let Some(sem) = sem_opt {
            let count = sem.inner.exclusive_access().count;
            available_sem[i] = if count > 0 { count as usize } else { 0 };
        }
    }

    // We need to gather all threads
    let mut all_tids = alloc::vec::Vec::new();
    for (&tid, _) in current_proc.mutex_allocations.iter() {
        if !all_tids.contains(&tid) { all_tids.push(tid); }
    }
    for (&tid, _) in current_proc.sem_allocations.iter() {
        if !all_tids.contains(&tid) { all_tids.push(tid); }
    }
    for (&tid, _) in current_proc.mutex_requests.iter() {
        if !all_tids.contains(&tid) { all_tids.push(tid); }
    }
    for (&tid, _) in current_proc.sem_requests.iter() {
        if !all_tids.contains(&tid) { all_tids.push(tid); }
    }
    if !all_tids.contains(&current_tid) { all_tids.push(current_tid); }

    let n = all_tids.len();
    
    // Allocation matrices
    let mut alloc_m = alloc::vec![alloc::vec![0; num_mutexes]; n];
    let mut alloc_s = alloc::vec![alloc::vec![0; num_sems]; n];
    // Need matrices (Actually request matrices)
    let mut need_m = alloc::vec![alloc::vec![0; num_mutexes]; n];
    let mut need_s = alloc::vec![alloc::vec![0; num_sems]; n];

    for (idx, &tid) in all_tids.iter().enumerate() {
        if let Some(m_list) = current_proc.mutex_allocations.get(&tid) {
            for &m_id in m_list.iter() { alloc_m[idx][m_id] += 1; }
        }
        if let Some(s_list) = current_proc.sem_allocations.get(&tid) {
            for &s_id in s_list.iter() { alloc_s[idx][s_id] += 1; }
        }
        if let Some(&req) = current_proc.mutex_requests.get(&tid) {
            need_m[idx][req] += 1;
        }
        if let Some(&req) = current_proc.sem_requests.get(&tid) {
            need_s[idx][req] += 1;
        }
        
        // Add current request
        if tid == current_tid {
            if let Some(m) = req_mutex { need_m[idx][m] += 1; }
            if let Some(s) = req_sem { need_s[idx][s] += 1; }
        }
    }

    let mut work_m = available_mutex.clone();
    let mut work_s = available_sem.clone();
    let mut finish = alloc::vec![false; n];

    loop {
        let mut found = false;
        for i in 0..n {
            if !finish[i] {
                // Check if need <= work
                let mut can_finish = true;
                for j in 0..num_mutexes {
                    if need_m[i][j] > work_m[j] { can_finish = false; break; }
                }
                if can_finish {
                    for j in 0..num_sems {
                        if need_s[i][j] > work_s[j] { can_finish = false; break; }
                    }
                }

                if can_finish {
                    // Reclaim resources
                    for j in 0..num_mutexes { work_m[j] += alloc_m[i][j]; }
                    for j in 0..num_sems { work_s[j] += alloc_s[i][j]; }
                    finish[i] = true;
                    found = true;
                }
            }
        }
        if !found { break; }
    }

    // If any is not finished, deadlock
    for i in 0..n {
        if !finish[i] {
            return true; // deadlock!
        }
    }

    false
}
