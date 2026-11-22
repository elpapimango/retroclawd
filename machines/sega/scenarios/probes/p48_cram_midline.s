; P48: a palette write in the middle of a line. The display is on with nothing drawn, so every pixel is colour 0 of palette 0 (the
; backdrop). The horizontal interrupt of every line jumps into a sled of NOPs (4 cycles each), skipping all but k of them, and writes
; colour 0 (CRAM entry 0) = $0E00 (blue); the line shows where along it the new colour starts. k grows by one each line, so the
; picture is a staircase whose edge moves right as the cycle count of the write grows; the next line's handler sets colour 0 black
; again first. Compare the picture, not RAM.
; mkrom: --vector 28=0x204
    .org $200
    bra.w   main
hint:
    addq.w  #1,d6
    cmp.w   #224,d6
    bcs.s   counted
    moveq   #0,d6
    move.w  #198,d5
counted:
    tst.w   d5
    bmi.w   out
    move.l  #$C0000000,$C00004
    move.w  #$0000,$C00000
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
    move.l  #$C0000000,$C00004
    move.w  #$0E00,$C00000
    subq.w  #2,d5
out:
    rte
main:
    lea     $C00004,a1
    move.w  #$8004,(a1)
    move.w  #$8014,(a1)
    move.w  #$8144,(a1)
    move.w  #$8A00,(a1)
    move.w  #$8C81,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8700,(a1)
    move.w  #198,d5
    moveq   #0,d6
sleep:
    stop    #$2000
    bra.s   sleep
