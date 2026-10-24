; P31 (p31g_fifo_vram_h32_d40): when the VDP's FIFO frees its entries. The 68000 sleeps in STOP; every line the horizontal interrupt handler
; waits 0 NOPs, writes four words to the data port (filling the FIFO), skips all but k of 88 NOPs and reads the status
; register's FIFO-full and FIFO-empty bits (bit 8 and 9). k = 0.. one more each line. Display on, H40.
; Result at RAM offset 0: 88 bytes of status bits 9:8 (1 = full, 2 = empty, 0 = between).
; mkrom: --vector 28=0x204
    .org $200
    bra.w   main
hint:
    tst.w   d5
    bmi.w   out
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    move.w  d2,(a0)
    jmp     run(pc,d5.w)
run:
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    move.w  $C00004,d4
    lsr.w   #8,d4
    and.b   #3,d4
    move.b  d4,(a2)+
    subq.w  #2,d5
out:
    rte
main:
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8014,(a1)
    move.w  #$8144,(a1)
    move.w  #$8A00,(a1)
    move.w  #$8C00,(a1)
    move.w  #$8F02,(a1)
    move.l  #$40000000,(a1)
    move.w  #$1234,d2
    move.w  #176,d5
sleep:
    stop    #$2000
    bra.s   sleep
