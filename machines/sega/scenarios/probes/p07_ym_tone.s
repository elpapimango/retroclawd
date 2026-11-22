; P07: one FM note: channel 1, algorithm 7, all four operators audible at TL 0, about 440 Hz, key on and held.
; The audio is compared between the emulators (level and pitch).
    .org $200
    move.w  #$0100,$A11100     ; take the Z80's bus
    move.w  #$0100,$A11200     ; and let it out of reset so the YM is reachable
wait:
    btst    #0,$A11100
    bne.s   wait
    lea     $A04000,a0
    ; operators: DT/MUL, TL, KS/AR, AM/DR, SR, SL/RR for each of the four
    moveq   #3,d3
    moveq   #0,d2              ; operator offset 0, 4, 8, 12
ops:
    move.b  #$30,d0
    add.b   d2,d0
    move.b  #$01,d1
    bsr.s   ym0
    move.b  #$40,d0
    add.b   d2,d0
    moveq   #0,d1
    bsr.s   ym0
    move.b  #$50,d0
    add.b   d2,d0
    move.b  #$1F,d1
    bsr.s   ym0
    move.b  #$60,d0
    add.b   d2,d0
    moveq   #0,d1
    bsr.s   ym0
    move.b  #$70,d0
    add.b   d2,d0
    moveq   #0,d1
    bsr.s   ym0
    move.b  #$80,d0
    add.b   d2,d0
    move.b  #$0F,d1
    bsr.s   ym0
    addq.b  #4,d2
    dbf     d3,ops
    move.b  #$B0,d0
    move.b  #$07,d1
    bsr.s   ym0
    move.b  #$B4,d0
    move.b  #$C0,d1
    bsr.s   ym0
    move.b  #$A4,d0
    move.b  #$22,d1
    bsr.s   ym0
    move.b  #$A0,d0
    move.b  #$69,d1
    bsr.s   ym0
    move.b  #$28,d0
    move.b  #$F0,d1
    bsr.s   ym0
forever:
    bra.s   forever
ym0:
    move.b  d0,(a0)
    moveq   #20,d4
y1: dbf     d4,y1
    move.b  d1,1(a0)
    moveq   #40,d4
y2: dbf     d4,y2
    rts
