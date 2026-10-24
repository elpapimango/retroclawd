; P08b: the cartridge area in 64 KB steps: the word read at every $10000 from $000000 to $3F0000, 64 words (the ROM is padded with a
; pattern whose byte is the 4 KB block number, so the word's value says which block of the ROM answered).
    .org $200
    lea     $FF0000,a2
    lea     $000000,a0
    move.w  #63,d0
l1: move.w  (a0),(a2)+
    adda.l  #$10000,a0
    dbf     d0,l1
forever:
    bra.s   forever
