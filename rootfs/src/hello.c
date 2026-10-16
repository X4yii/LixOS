typedef unsigned long long uint64_t;
static inline uint64_t syscall3(uint64_t sys_num, uint64_t arg1, uint64_t arg2, uint64_t arg3) {
    uint64_t ret;
    __asm__ volatile (
        "int $0x80"
        : "=a"(ret)
        : "a"(sys_num), "D"(arg1), "S"(arg2), "d"(arg3)
        : "rcx", "r11", "memory"
    );
    return ret;
}
static inline uint64_t syscall1(uint64_t sys_num, uint64_t arg1) {
    uint64_t ret;
    __asm__ volatile (
        "int $0x80"
        : "=a"(ret)
        : "a"(sys_num), "D"(arg1)
        : "rcx", "r11", "memory"
    );
    return ret;
}
void print(const char* str) {
    uint64_t len = 0;
    while (str[len]) len++;
    syscall3(1, 1, (uint64_t)str, len);
}
void exit(int code) {
    syscall1(60, (uint64_t)code);
    while (1);
}
void _start() {
    print("\n[Ring 3] Hola desde el Userspace\n");
    print("LixOS esta ejecutando un binario ELF compilado en C nativo. del tamaño de tu madre\n");
    print("bien joto el q lo lea, bue es un OS q ando programando\n\n");
    exit(42);
}