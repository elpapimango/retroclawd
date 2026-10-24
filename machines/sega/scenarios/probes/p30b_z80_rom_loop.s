; P30: what a Z80 reading the 68000's bus costs the 68000, and what the Z80 itself manages. The Z80 runs a loop that reads
; (a) its own RAM or (b) the ROM through the bank window (the two probes differ only in the Z80 program). The 68000 counts the
; iterations of a 42-cycle loop between V = 60 and V = 80 (display on, 20 lines), then waits a fixed time and reads how far the
; Z80 loop got.
; Result at RAM offset 0: word 68000 iterations, word Z80 iterations.
    .org $200
    lea     $FF0000,a2
    lea     $C00004,a1
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    move.w  #$0100,$A11100
    move.w  #$0100,$A11200
    move.w  #2000,d0
g1: btst    #0,$A11100
    beq.s   g2
    dbf     d0,g1
g2: lea     z80code(pc),a0
    lea     $A00000,a3
    move.w  #(z80end-z80code)-1,d0
cp: move.b  (a0)+,(a3)+
    dbf     d0,cp
    move.w  #$0000,$A11200
    moveq   #50,d0
rd: dbf     d0,rd
    move.w  #$0100,$A11200
    move.w  #$0000,$A11100
    move.l  #3000,d0
wt0: subq.l #1,d0
    bne.s   wt0
w1: move.b  $C00008,d3
    cmp.b   #60,d3
    bne.s   w1
    moveq   #0,d7
w2: addq.l  #1,d7
    move.b  $C00008,d3
    cmp.b   #80,d3
    bne.s   w2
    move.w  d7,(a2)+
    move.l  #20000,d0
wt: subq.l  #1,d0
    bne.s   wt
    move.w  #$0100,$A11100
    move.w  #2000,d0
g3: btst    #0,$A11100
    beq.s   g4
    dbf     d0,g3
g4: move.b  $A01F01,d1
    lsl.w   #8,d1
    move.b  $A01F00,d1
    move.w  d1,(a2)+
forever:
    bra.s   forever
z80code:
    .include "p30b_z80_rom_loop.z80.inc"
z80end:
