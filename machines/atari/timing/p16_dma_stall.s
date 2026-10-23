; p16_dma_stall.s: does a floppy DMA transfer slow the CPU down?
; A counted loop runs while the floppy controller reads nine sectors into $20000 (the command is given in frame 3).
; Every VBL stores 8 bytes: the loop count of the frame just ended (long), the MFP GPIP byte ($FFFA01, bit 5 low =
; the controller's interrupt request), the DMA status ($FF8606 low byte), the DMA address counter (mid, low).
; Result at $C000: header 'P16', then 30 records of 8 bytes (frames 2 to 31).
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     vbl(pc),a0
    move.l  a0,$70
    lea     $c000,a2
    move.l  #$50313600,(a2)+
    moveq   #0,d5               ; loop count
    moveq   #0,d6               ; frames
    move.w  #$2300,sr
wait:
    cmp.w   #3,d6
    blt.s   wait
    move.w  #$190,$ff8606       ; toggling the write bit clears the FIFO
    move.w  #$090,$ff8606       ; read, the sector count register
    move.w  #9,$ff8604
    move.b  #$02,$ff8609        ; DMA address $020000, low byte first
    move.b  #$00,$ff860b
    move.b  #$00,$ff860d
    move.w  #$084,$ff8606       ; the sector register
    move.w  #1,$ff8604
    move.w  #$080,$ff8606       ; the command register
    move.w  #$90,$ff8604        ; read sector, multiple
    moveq   #0,d5
loop:
    addq.l  #1,d5
    bra.s   loop

vbl:
    addq.w  #1,d6
    cmp.w   #2,d6
    blt.s   clear
    move.l  d5,(a2)+
    move.b  $fffa01,(a2)+
    move.w  $ff8606,d0
    move.b  d0,(a2)+
    move.b  $ff860b,(a2)+
    move.b  $ff860d,(a2)+
    cmp.w   #32,d6
    blt.s   clear
    ori.w   #$0700,(sp)
clear:
    moveq   #0,d5
    rte
