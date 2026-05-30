use super::*;

// ---------------------------------------------------------------------------
// Agent handle table
// ---------------------------------------------------------------------------

pub(crate) fn take_runtime(handle: u32) -> Result<(AgentRuntime, SessionState), HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        slots[idx].take().ok_or(HostError::BadHandle(handle))
    })
}

pub(crate) fn put_runtime(runtime: AgentRuntime, session_state: SessionState) -> u32 {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some((runtime, session_state));
                return i as u32;
            }
        }
        let handle = slots.len() as u32;
        slots.push(Some((runtime, session_state)));
        handle
    })
}

pub(crate) fn with_runtime<T>(handle: u32, op: impl FnOnce(&mut AgentRuntime, &mut SessionState) -> T) -> Result<T, HostError> {
    AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        match &mut slots[idx] {
            Some((runtime, session_state)) => Ok(op(runtime, session_state)),
            None => Err(HostError::BadHandle(handle)),
        }
    })
}

// ---------------------------------------------------------------------------
// HostState handle table
// ---------------------------------------------------------------------------

pub(crate) fn take_host_state(handle: u32) -> Result<HostState, HostError> {
    HOST_STATE_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        slots[idx].take().ok_or(HostError::BadHandle(handle))
    })
}

pub(crate) fn put_host_state(state: HostState) -> u32 {
    HOST_STATE_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(state);
                return i as u32;
            }
        }
        let handle = slots.len() as u32;
        slots.push(Some(state));
        handle
    })
}

pub(crate) fn with_host_state<T>(handle: u32, op: impl FnOnce(&mut HostState) -> T) -> Result<T, HostError> {
    HOST_STATE_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        match &mut slots[idx] {
            Some(state) => Ok(op(state)),
            None => Err(HostError::BadHandle(handle)),
        }
    })
}

// ---------------------------------------------------------------------------
// HostAgent — combines AgentRuntime + HostState for the directive-based API
// ---------------------------------------------------------------------------

pub(crate) struct HostAgent {
    pub(crate) runtime: AgentRuntime,
    pub(crate) host_state: HostState,
    pub(crate) session_state: SessionState,
}

pub(crate) fn take_host_agent(handle: u32) -> Result<HostAgent, HostError> {
    HOST_AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        slots[idx].take().ok_or(HostError::BadHandle(handle))
    })
}

pub(crate) fn put_host_agent(agent: HostAgent) -> u32 {
    HOST_AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(agent);
                return i as u32;
            }
        }
        let handle = slots.len() as u32;
        slots.push(Some(agent));
        handle
    })
}

pub(crate) fn with_host_agent<T>(handle: u32, op: impl FnOnce(&mut HostAgent) -> T) -> Result<T, HostError> {
    HOST_AGENT_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let idx = handle as usize;
        if idx >= slots.len() {
            return Err(HostError::BadHandle(handle));
        }
        match &mut slots[idx] {
            Some(agent) => Ok(op(agent)),
            None => Err(HostError::BadHandle(handle)),
        }
    })
}
