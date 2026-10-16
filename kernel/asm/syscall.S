.intel_syntax noprefix
.text

.global syscall_entry_asm
.type syscall_entry_asm, @function
syscall_entry_asm:
    swapgs
    mov qword ptr gs:[8], rsp
    mov rsp, qword ptr gs:[0]
    push qword ptr gs:[8]
    push r11
    push rcx
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9
    sub rsp, 8
    mov rdi, rax
    mov rsi, [rsp + 48]
    mov rdx, [rsp + 40]
    mov rcx, [rsp + 32]
    mov r8,  [rsp + 24]
    mov r9,  [rsp + 16]
    cld
    sti
    call syscall_dispatcher
    cli
    add rsp, 8
    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    pop rcx
    pop r11
    cli
    pop rsp
    swapgs
    sysretq

.global syscall_handler_asm
.type syscall_handler_asm, @function
syscall_handler_asm:
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    and rsp, -16
    cld
    call syscall_handler_rust
    mov rsp, rax
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    iretq
