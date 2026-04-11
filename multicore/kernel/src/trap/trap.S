.altmacro
.set REG_SIZE, 8

.macro SAVE_GP n
    sd x\n, \n*REG_SIZE(sp)
.endm

.macro LOAD_GP n
    ld x\n, \n*REG_SIZE(sp)
.endm

    .section .text
    .globl user_trap_vector
    .globl user_trap_return
    .globl kernel_trap_vector

# =======================================================
# User trap vector: entered from U-mode
# =======================================================
    .align 4
user_trap_vector:
    csrrw sp, sscratch, sp

    sd x1, 1*REG_SIZE(sp)
    sd x3, 3*REG_SIZE(sp)
    sd x4, 4*REG_SIZE(sp)
    .set n, 5
    .rept 27
        SAVE_GP %n
        .set n, n+1
    .endr

    csrr t0, sscratch
    sd t0, 2*REG_SIZE(sp)

    csrr t0, sstatus
    sd t0, 32*REG_SIZE(sp)
    csrr t1, sepc
    sd t1, 33*REG_SIZE(sp)

    ld t0, 34*REG_SIZE(sp)     # kernel_satp
    ld t1, 35*REG_SIZE(sp)     # kernel_sp
    ld t2, 36*REG_SIZE(sp)     # trap_handler

    csrw satp, t0
    sfence.vma

    mv sp, t1
    la t3, kernel_trap_vector
    csrw stvec, t3
    jr t2

# =======================================================
# User trap return
# a0 = TrapContext ptr, a1 = user satp
# =======================================================
    .align 4
user_trap_return:
    la t0, user_trap_vector
    csrw stvec, t0

    # Save user_satp in sscratch temporarily
    csrw sscratch, a1

    mv sp, a0

    # Restore sstatus and sepc
    ld t0, 32*REG_SIZE(sp)
    csrw sstatus, t0
    ld t1, 33*REG_SIZE(sp)
    csrw sepc, t1

    # Restore all GPRs EXCEPT x2(sp), x5(t0), x6(t1)
    # x5 and x6 are deferred because we need them as scratch
    # registers for the satp switch and sp restore below.
    ld x1, 1*REG_SIZE(sp)
    ld x3, 3*REG_SIZE(sp)
    .set n, 7
    .rept 25
        LOAD_GP %n
        .set n, n+1
    .endr

    # Use t0 and t1 as scratch for satp switch and sp restore
    ld t0, 2*REG_SIZE(sp)          # t0 = user_sp
    csrr t1, sscratch              # t1 = user_satp

    # Set sscratch to TrapContext address (for next trap)
    csrw sscratch, sp

    # Switch to user page table
    csrw satp, t1
    sfence.vma

    # Set sp to user sp
    mv sp, t0

    # Finally restore x5(t0) and x6(t1) from trap context.
    # sscratch still points to the TrapContext physical page,
    # which is accessible via the kernel identity mapping in
    # the user page table.
    csrr t0, sscratch
    ld t0, 5*REG_SIZE(t0)          # x5 = user's t0
    csrr t1, sscratch
    ld t1, 6*REG_SIZE(t1)          # x6 = user's t1

    sret

# =======================================================
# Kernel trap vector
# =======================================================
    .align 4
kernel_trap_vector:
    addi sp, sp, -16*REG_SIZE

    sd ra,  0*REG_SIZE(sp)
    sd s0,  2*REG_SIZE(sp)
    sd s1,  3*REG_SIZE(sp)
    sd s2,  4*REG_SIZE(sp)
    sd s3,  5*REG_SIZE(sp)
    sd s4,  6*REG_SIZE(sp)
    sd s5,  7*REG_SIZE(sp)
    sd s6,  8*REG_SIZE(sp)
    sd s7,  9*REG_SIZE(sp)
    sd s8,  10*REG_SIZE(sp)
    sd s9,  11*REG_SIZE(sp)
    sd s10, 12*REG_SIZE(sp)
    sd s11, 13*REG_SIZE(sp)

    csrr t0, sstatus
    sd t0, 14*REG_SIZE(sp)
    csrr t1, sepc
    sd t1, 15*REG_SIZE(sp)

    addi t2, sp, 16*REG_SIZE
    sd t2, 1*REG_SIZE(sp)

    csrr a0, stval
    csrr a1, scause
    mv a2, t1
    call kernel_trap_handler

    ld t0, 14*REG_SIZE(sp)
    csrw sstatus, t0
    ld t1, 15*REG_SIZE(sp)
    csrw sepc, t1

    ld ra,  0*REG_SIZE(sp)
    ld s0,  2*REG_SIZE(sp)
    ld s1,  3*REG_SIZE(sp)
    ld s2,  4*REG_SIZE(sp)
    ld s3,  5*REG_SIZE(sp)
    ld s4,  6*REG_SIZE(sp)
    ld s5,  7*REG_SIZE(sp)
    ld s6,  8*REG_SIZE(sp)
    ld s7,  9*REG_SIZE(sp)
    ld s8,  10*REG_SIZE(sp)
    ld s9,  11*REG_SIZE(sp)
    ld s10, 12*REG_SIZE(sp)
    ld s11, 13*REG_SIZE(sp)

    addi sp, sp, 16*REG_SIZE
    sret
