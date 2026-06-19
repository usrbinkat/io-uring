use crate::Test;
use io_uring::{cqueue, opcode, squeue, IoUring};

pub fn test_register_files_sparse<S: squeue::EntryMarker, C: cqueue::EntryMarker>(
    ring: &mut IoUring<S, C>,
    test: &Test,
) -> anyhow::Result<()> {
    // register_files_sparse was introduced in kernel 5.19, as was the opcode for UringCmd16.
    // So require the UringCmd16 to avoid running this test on earlier kernels.
    require!(
        test;
        test.probe.is_supported(opcode::UringCmd16::CODE);
    );

    println!("test register_files_sparse");

    ring.submitter().register_files_sparse(4)?;

    // See that same call again, with any value, will fail because a direct table cannot be built
    // over an existing one.

    if let Ok(()) = ring.submitter().register_files_sparse(3) {
        return Err(anyhow::anyhow!(
            "register_files_sparse should not have succeeded twice in a row"
        ));
    }

    // See that the direct table can be removed.

    if let Err(e) = ring.submitter().unregister_files() {
        return Err(anyhow::anyhow!("unrgister_files failed: {}", e));
    }

    // See that a second attempt to remove the direct table would fail.

    if let Ok(()) = ring.submitter().unregister_files() {
        return Err(anyhow::anyhow!(
            "unrgister_files should not have succeeded twice in a row"
        ));
    }

    // See that a new, large, direct table can be created.
    // If it fails with EMFILE, print the ulimit command for changing this.

    if let Err(e) = ring.submitter().register_files_sparse(10_000) {
        if let Some(raw_os_err) = e.raw_os_error() {
            if raw_os_err == libc::EMFILE {
                println!(
                    "could not open 10,000 file descriptors, try `ulimit -Sn 11000` in the shell"
                );
                return Ok(());
            } else {
                return Err(anyhow::anyhow!("register_files_sparse should have succeeded after the previous one was removed: {}", e));
            }
        } else {
            return Err(anyhow::anyhow!("register_files_sparse should have succeeded after the previous one was removed: {}", e));
        }
    }

    // And removed.

    if let Err(e) = ring.submitter().unregister_files() {
        return Err(anyhow::anyhow!(
            "unrgister_files failed, odd since the one could be unregistered earlier: {}",
            e
        ));
    }

    Ok(())
}

pub fn test_register_ring_fd<S: squeue::EntryMarker, C: cqueue::EntryMarker>(
    ring: &mut IoUring<S, C>,
    test: &Test,
) -> anyhow::Result<()> {
    require!(
        test;
        ring.params().is_feature_reg_reg_ring();
    );

    println!("test register_ring_fd");

    // Register the ring fd
    let idx = ring.register_ring_fd()?;
    println!("  registered ring fd at index {idx}");

    // Double registration must fail — ring_fd_registered is already Some
    // IoUring::register_ring_fd checks stored state before calling the kernel.
    assert!(
        ring.register_ring_fd().is_err(),
        "double register_ring_fd should fail"
    );

    // Submit a NOP via submit_and_wait to verify the registered path works
    // through the IoUring-level convenience method (creates temporary Submitter)
    let nop = opcode::Nop::new().build().user_data(0xA1);
    unsafe { ring.submission().push(&nop.into())? };
    ring.submit_and_wait(1)?;
    let cqe: cqueue::Entry = ring.completion().next().unwrap().into();
    assert_eq!(cqe.user_data(), 0xA1);
    assert_eq!(cqe.result(), 0);

    // Submit a NOP via split() submitter to verify the registered path works
    // through a long-lived Submitter (the pattern used by uring task loops)
    {
        let (submitter, mut sq, mut cq) = ring.split();
        let nop = opcode::Nop::new().build().user_data(0xA3).into();
        unsafe { sq.push(&nop)? };
        sq.sync();
        submitter.submit_and_wait(1)?;
        cq.sync();
        let cqe: cqueue::Entry = cq.next().unwrap().into();
        assert_eq!(cqe.user_data(), 0xA3);
        assert_eq!(cqe.result(), 0);
    }

    // Unregister
    ring.unregister_ring_fd()?;

    // Double unregister must succeed silently (no-op when already None)
    ring.unregister_ring_fd()?;

    // Submit another NOP to verify fallback to normal fd path works
    let nop = opcode::Nop::new().build().user_data(0xA2);
    unsafe { ring.submission().push(&nop.into())? };
    ring.submit_and_wait(1)?;
    let cqe: cqueue::Entry = ring.completion().next().unwrap().into();
    assert_eq!(cqe.user_data(), 0xA2);
    assert_eq!(cqe.result(), 0);

    // Re-register after unregister must work
    let idx2 = ring.register_ring_fd()?;
    println!("  re-registered ring fd at index {idx2}");
    let nop = opcode::Nop::new().build().user_data(0xA4);
    unsafe { ring.submission().push(&nop.into())? };
    ring.submit_and_wait(1)?;
    let cqe: cqueue::Entry = ring.completion().next().unwrap().into();
    assert_eq!(cqe.user_data(), 0xA4);
    assert_eq!(cqe.result(), 0);

    // Clean up
    ring.unregister_ring_fd()?;

    Ok(())
}
