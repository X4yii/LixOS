.intel_syntax noprefix
.text

.global timer_interrupt_handler_asm
.type timer_interrupt_handler_asm, @function
timer_interrupt_handler_asm:
    test byte ptr [rsp + 8], 3
    jz 1f
    swapgs
1:
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
    call timer_interrupt_handler_rust
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

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global reschedule_interrupt_handler_asm
.type reschedule_interrupt_handler_asm, @function
reschedule_interrupt_handler_asm:
    test byte ptr [rsp + 8], 3
    jz 1f
    swapgs
1:
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
    call reschedule_interrupt_handler_rust
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

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global keyboard_interrupt_handler_asm
.type keyboard_interrupt_handler_asm, @function
keyboard_interrupt_handler_asm:
    test byte ptr [rsp + 8], 3
    jz 1f
    swapgs
1:
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
    call keyboard_interrupt_handler_rust
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

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global page_fault_handler_asm
.type page_fault_handler_asm, @function
page_fault_handler_asm:
    test byte ptr [rsp + 16], 3
    jz 1f
    swapgs
1:
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
    sub rsp, 8
    cld
    call page_fault_handler_rust
    add rsp, 8

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
    add rsp, 8

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global gpf_handler_asm
.type gpf_handler_asm, @function
gpf_handler_asm:
    test byte ptr [rsp + 16], 3
    jz 1f
    swapgs
1:
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
    sub rsp, 8
    cld
    call gpf_handler_rust
    add rsp, 8

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
    add rsp, 8

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global double_fault_handler_asm
.type double_fault_handler_asm, @function
double_fault_handler_asm:
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
    sub rsp, 8
    cld
    call double_fault_handler_rust
    add rsp, 8
    ud2

.global divide_error_handler_asm
.type divide_error_handler_asm, @function
divide_error_handler_asm:
    push 0
    test byte ptr [rsp + 16], 3
    jz 1f
    swapgs
1:
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
    sub rsp, 8
    cld
    call divide_error_handler_rust
    add rsp, 8

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
    add rsp, 8

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global invalid_opcode_handler_asm
.type invalid_opcode_handler_asm, @function
invalid_opcode_handler_asm:
    push 0
    test byte ptr [rsp + 16], 3
    jz 1f
    swapgs
1:
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
    sub rsp, 8
    cld
    call invalid_opcode_handler_rust
    add rsp, 8

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
    add rsp, 8

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq

.global breakpoint_handler_asm
.type breakpoint_handler_asm, @function
breakpoint_handler_asm:
    push 0
    test byte ptr [rsp + 16], 3
    jz 1f
    swapgs
1:
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
    sub rsp, 8
    cld
    call breakpoint_handler_rust
    add rsp, 8

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
    add rsp, 8

    test byte ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq
