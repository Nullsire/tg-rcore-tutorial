use crate::event::{Event, EventType};
use crate::mutex::PiMutex;
use crate::scheduler::EdfScheduler;
use crate::task::{Task, TaskState};

/// Tick-based discrete-event RTOS kernel simulator.
pub struct Kernel {
    pub tasks: Vec<Task>,
    pub mutexes: Vec<PiMutex>,
    pub current_time: u32,
    pub sim_duration: u32,
    pub events: Vec<Event>,
    /// Which task was running at each tick (for timeline).
    pub tick_runner: Vec<Option<usize>>,
    running_task_id: Option<usize>,
}

impl Kernel {
    pub fn new(tasks: Vec<Task>, mutexes: Vec<PiMutex>, sim_duration: u32) -> Self {
        Kernel {
            tasks,
            mutexes,
            current_time: 0,
            sim_duration,
            events: Vec::new(),
            tick_runner: Vec::with_capacity(sim_duration as usize),
            running_task_id: None,
        }
    }

    /// Run the simulation to completion.
    pub fn run(&mut self) {
        while self.current_time < self.sim_duration {
            self.tick();
            self.current_time += 1;
        }
    }

    fn tick(&mut self) {
        let t = self.current_time;

        // 0. Reset currently running task to Ready for rescheduling
        let prev_running = self.running_task_id;
        if let Some(cur) = self.running_task_id {
            if self.tasks[cur].state == TaskState::Running {
                self.tasks[cur].state = TaskState::Ready;
            }
        }

        // 1. Release new periodic jobs
        for i in 0..self.tasks.len() {
            if self.tasks[i].next_release == t {
                // If previous job not done, count as deadline miss
                if self.tasks[i].state != TaskState::Suspended {
                    self.tasks[i].deadline_misses += 1;
                    if let Some(mux_id) = self.tasks[i].held_mutex {
                        self.mutexes[mux_id].unlock(
                            i,
                            &mut self.tasks,
                            &mut self.events,
                            t,
                        );
                    }
                    self.tasks[i].state = TaskState::Suspended;
                    let rt = t - self.tasks[i].current_job_release;
                    self.tasks[i].response_times.push(rt);
                    self.tasks[i].jobs_completed += 1;
                    self.events.push(Event::new(
                        t,
                        i,
                        EventType::DeadlineMissed {
                            deadline: self.tasks[i].absolute_deadline,
                            completed_at: t,
                        },
                    ));
                }
                self.tasks[i].release_job(t);
                self.events.push(Event::new(t, i, EventType::Released));

                // If first segment is a critical section, try to acquire mutex
                let first_mutex = self.tasks[i].current_segment_mutex();
                if let Some(mutex_id) = first_mutex {
                    if self.tasks[i].held_mutex.is_none() {
                        self.mutexes[mutex_id].try_lock(
                            i,
                            &mut self.tasks,
                            &mut self.events,
                            t,
                        );
                    }
                }
            }
        }

        // 2. Schedule: pick the ready task with earliest effective deadline
        let next = EdfScheduler::pick_next(&self.tasks);

        if let Some(next_id) = next {
            // Log preemption if different task chosen and prev was still Running
            if let Some(prev) = prev_running {
                if prev != next_id && self.tasks[prev].state == TaskState::Ready {
                    self.events.push(Event::new(t, prev, EventType::Preempted));
                }
            }

            self.tasks[next_id].state = TaskState::Running;
            self.running_task_id = Some(next_id);
            self.events.push(Event::new(t, next_id, EventType::Scheduled));

            // 3. Execute one tick
            self.tasks[next_id].segment_remaining -= 1;

            // 4. Check segment completion
            if self.tasks[next_id].segment_remaining == 0 {
                self.handle_segment_done(next_id);
            }

            self.tick_runner.push(Some(next_id));
        } else {
            // No task to run — idle tick
            self.running_task_id = None;
            self.tick_runner.push(None);
        }
    }

    /// Handle completion of the current segment for a task.
    fn handle_segment_done(&mut self, task_id: usize) {
        // Step 1: Get current segment's mutex (if any) — returns Copy value
        let cur_mutex = self.tasks[task_id].current_segment_mutex();

        // Step 2: Release mutex if we were in a critical section
        if let Some(mutex_id) = cur_mutex {
            self.mutexes[mutex_id].unlock(
                task_id,
                &mut self.tasks,
                &mut self.events,
                self.current_time,
            );
        }

        // Step 3: Advance to next segment
        self.tasks[task_id].advance_to_next_segment();

        // Step 4: Check if job is complete
        if self.tasks[task_id].is_job_complete() {
            self.handle_job_complete(task_id);
            return;
        }

        // Step 5: If new segment needs a mutex, try to acquire
        let next_mutex = self.tasks[task_id].current_segment_mutex();
        if let Some(mutex_id) = next_mutex {
            self.mutexes[mutex_id].try_lock(
                task_id,
                &mut self.tasks,
                &mut self.events,
                self.current_time,
            );
        }
    }

    /// Handle job completion for a task.
    fn handle_job_complete(&mut self, task_id: usize) {
        let completion_time = self.current_time;
        let deadline = self.tasks[task_id].absolute_deadline;
        let release_time = self.tasks[task_id].current_job_release;
        let response_time = completion_time + 1 - release_time;

        self.tasks[task_id].response_times.push(response_time);
        self.tasks[task_id].jobs_completed += 1;
        self.tasks[task_id].state = TaskState::Suspended;

        if completion_time + 1 > deadline {
            self.tasks[task_id].deadline_misses += 1;
            self.events.push(Event::new(
                completion_time,
                task_id,
                EventType::DeadlineMissed {
                    deadline,
                    completed_at: completion_time + 1,
                },
            ));
        }

        self.events.push(Event::new(
            completion_time,
            task_id,
            EventType::JobCompleted { response_time },
        ));

        if self.running_task_id == Some(task_id) {
            self.running_task_id = None;
        }
    }

    pub fn task_name(&self, id: usize) -> &str {
        &self.tasks[id].name
    }
}
