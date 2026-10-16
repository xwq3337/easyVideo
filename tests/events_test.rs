use easydownvideo::core::events::ProgressHandle;

#[test]
fn progress_handle_tracks_counts() {
    let p = ProgressHandle::new();
    assert_eq!(p.snapshot(), (0, 0));
    p.set_total(1000);
    p.add(250);
    p.add(250);
    assert_eq!(p.snapshot(), (500, 1000));
    p.reset();
    assert_eq!(p.snapshot(), (0, 1000));
}

#[test]
fn progress_handle_unit_defaults_to_bytes() {
    use easydownvideo::core::events::ProgressUnit;
    let p = ProgressHandle::new();
    assert_eq!(p.unit(), ProgressUnit::Bytes);
    p.set_unit(ProgressUnit::Segments);
    assert_eq!(p.unit(), ProgressUnit::Segments);
}

#[test]
fn sub_rolls_back_without_underflow() {
    let p = ProgressHandle::new();
    p.set_total(1000);
    p.add(300);
    p.sub(120);
    assert_eq!(p.snapshot(), (180, 1000));
    // 回滚量超过已计入时饱和到 0，绝不回绕成天文数字
    p.sub(999);
    assert_eq!(p.snapshot(), (0, 1000));
}

/// 分片任务的速度必须是**字节/秒**，所以字节计数不能与"分片计数"共用同一个原子量：
/// `add()` 对分片任务是 +1 片，拿它当字节算出来的速度就是"片/秒"，混进总速度里
/// 会变成 "3.0 B/s" 这种假数字。两者分开记，`reset()` 一起归零。
#[test]
fn progress_handle_counts_bytes_separately_from_the_unit_counter() {
    use easydownvideo::core::events::ProgressUnit;
    let p = ProgressHandle::new();
    p.set_unit(ProgressUnit::Segments);
    p.add(3); // 3 个分片
    p.add_bytes(1_500_000);
    assert_eq!(p.snapshot(), (3, 0), "分片计数不该被字节数污染");
    assert_eq!(p.bytes(), 1_500_000);

    // 重跑（retry / unpause / 重启恢复）时两个计数一起从零开始
    p.reset();
    assert_eq!(p.snapshot(), (0, 0));
    assert_eq!(p.bytes(), 0);
}

/// 与 `sub` 对称：失败分块的重试必须能把字节数也退回去，且同样饱和到 0 不回绕。
#[test]
fn sub_bytes_rolls_back_without_underflow() {
    let p = ProgressHandle::new();
    p.add_bytes(300);
    p.sub_bytes(120);
    assert_eq!(p.bytes(), 180);
    p.sub_bytes(999);
    assert_eq!(p.bytes(), 0);
}

#[test]
fn task_state_variants_exist() {
    use easydownvideo::core::events::TaskState;
    let states = [
        TaskState::Queued,
        TaskState::Probing,
        TaskState::AwaitingChoice,
        TaskState::Downloading,
        TaskState::Merging,
        TaskState::Paused,
        TaskState::Done,
        TaskState::Failed("x".into()),
        TaskState::Canceled,
    ];
    assert_eq!(states.len(), 9);
}
