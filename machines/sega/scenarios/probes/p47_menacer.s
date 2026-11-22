; P47: a Menacer in port 2 pointing at one place. The port's HL pin is enabled, the VDP latches its counters on the line the gun
; points at and raises the external interrupt. After a few frames the HV counter is read (it stays latched, and the V counter cannot be
; used to wait: it is the latched one too); the external interrupt count is kept by the handler.
; mkrom: --vector 26=0x204
; Result at RAM offset 0: the HV counter twice (the latch holds it), port 2 data read twice; at $10 the interrupt count.
    .org $200
    bra.w   main
ext:
    addq.w  #1,$FF0010
    rte
main:
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8144,(a1)
    move.w  #$8C81,(a1)
    move.w  #$8006,(a1)
    move.w  #$8B08,(a1)
    move.b  #$80,$A1000B
    move.w  #0,$FF0010
    andi.w  #$F8FF,sr
    move.l  #40000,d0
w1: subq.l  #1,d0
    bne.s   w1
    move.w  $C00008,(a2)+
    move.w  $C00008,(a2)+
    move.b  $A10005,d0
    move.b  d0,(a2)+
    move.b  d0,(a2)+
forever:
    bra.s   forever
